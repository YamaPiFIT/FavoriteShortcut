using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

public readonly record struct SearchHit(ShortcutItem Item, int Score);

/// <summary>
/// タイトル / タグ / フォルダ名 / URL・パス / メモ を横断する検索（§12〜§15, §55）。
///
/// 全件がメモリ上にあるため、SQL ではなくメモリ上でスコアリングする。
/// 1万件程度でも 1 回の検索は数ミリ秒で終わる。
/// 複数ワードは AND 条件（すべての語がどこかに一致する必要がある）。
/// </summary>
public static class SearchService
{
    // スコアの基準値。数字自体に意味はなく、相対的な順位付けのためのもの。
    private const int ScoreTitleExact = 1000;
    private const int ScoreTitlePrefix = 700;
    private const int ScoreTitleContains = 500;
    private const int ScoreTagExact = 460;
    private const int ScoreTagPrefix = 380;
    private const int ScoreTagContains = 320;
    private const int ScoreFolderExact = 300;
    private const int ScoreFolderContains = 240;
    private const int ScoreTargetHostPrefix = 260;
    private const int ScoreTargetContains = 200;
    private const int ScoreNoteContains = 90;

    /// <summary>使用頻度・最近使ったかによる加点の上限。検索語の一致より優先されすぎないよう抑えめにする。</summary>
    private const int MaxUsageBonus = 150;

    /// <summary>
    /// 検索してスコア順に並べた結果を返す。query が空なら null（呼び出し側で既定の並びを使う）。
    /// </summary>
    public static List<SearchHit>? Search(
        IEnumerable<ShortcutItem> source, string? query, int revision, int maxResults = int.MaxValue)
    {
        var terms = TextNormalizer.SplitTerms(query);
        if (terms.Length == 0) return null;

        var hits = new List<SearchHit>();
        foreach (var item in source)
        {
            var index = GetIndex(item, revision);
            var total = 0;
            var matchedAll = true;

            foreach (var term in terms)
            {
                var best = ScoreTerm(index, term);
                if (best <= 0) { matchedAll = false; break; }
                total += best;
            }

            if (!matchedAll) continue;
            hits.Add(new SearchHit(item, total + UsageBonus(item)));
        }

        hits.Sort(CompareHits);
        if (hits.Count > maxResults) hits.RemoveRange(maxResults, hits.Count - maxResults);
        return hits;
    }

    private static int CompareHits(SearchHit a, SearchHit b)
    {
        var byScore = b.Score.CompareTo(a.Score);
        if (byScore != 0) return byScore;

        // 同スコアなら短いタイトルを先に（より「ぴったり」に見えるため）
        var byLength = a.Item.DisplayTitle.Length.CompareTo(b.Item.DisplayTitle.Length);
        if (byLength != 0) return byLength;

        return string.Compare(a.Item.DisplayTitle, b.Item.DisplayTitle, StringComparison.CurrentCultureIgnoreCase);
    }

    private static int ScoreTerm(SearchIndexEntry index, string term)
    {
        var best = 0;

        if (index.Title.Length > 0)
        {
            if (index.Title == term) best = Math.Max(best, ScoreTitleExact);
            else if (index.Title.StartsWith(term, StringComparison.Ordinal)) best = Math.Max(best, ScoreTitlePrefix);
            else if (index.Title.Contains(term, StringComparison.Ordinal)) best = Math.Max(best, ScoreTitleContains);
        }

        foreach (var tag in index.Tags)
        {
            if (tag == term) { best = Math.Max(best, ScoreTagExact); break; }
            if (tag.StartsWith(term, StringComparison.Ordinal)) best = Math.Max(best, ScoreTagPrefix);
            else if (tag.Contains(term, StringComparison.Ordinal)) best = Math.Max(best, ScoreTagContains);
        }

        if (index.FolderPath.Length > 0 && index.FolderPath.Contains(term, StringComparison.Ordinal))
        {
            // 「開発」のようにフォルダ名そのものと一致する場合は少し高く
            var exactSegment = index.FolderSegments.Any(s => s == term);
            best = Math.Max(best, exactSegment ? ScoreFolderExact : ScoreFolderContains);
        }

        if (index.Target.Length > 0 && index.Target.Contains(term, StringComparison.Ordinal))
        {
            var hostPrefix = index.Host.Length > 0 && index.Host.StartsWith(term, StringComparison.Ordinal);
            best = Math.Max(best, hostPrefix ? ScoreTargetHostPrefix : ScoreTargetContains);
        }

        if (index.Note.Length > 0 && index.Note.Contains(term, StringComparison.Ordinal))
            best = Math.Max(best, ScoreNoteContains);

        return best;
    }

    /// <summary>よく使う/最近使ったものを少しだけ上位に寄せる（§57）。</summary>
    private static int UsageBonus(ShortcutItem item)
    {
        var bonus = Math.Min(item.UsageCount, 30) * 3;

        if (item.LastUsedAt is { } last)
        {
            var elapsed = DateTime.UtcNow - last;
            if (elapsed < TimeSpan.FromHours(1)) bonus += 60;
            else if (elapsed < TimeSpan.FromDays(1)) bonus += 40;
            else if (elapsed < TimeSpan.FromDays(7)) bonus += 20;
        }

        return Math.Min(bonus, MaxUsageBonus);
    }

    /// <summary>最近使った順（ランチャーで検索語が空のとき用, §56）。</summary>
    public static List<ShortcutItem> Recent(IEnumerable<ShortcutItem> source, int maxResults)
    {
        return source
            .Where(s => s.LastUsedAt is not null)
            .OrderByDescending(s => s.LastUsedAt)
            .Take(maxResults)
            .ToList();
    }

    private static SearchIndexEntry GetIndex(ShortcutItem item, int revision)
    {
        var index = item.SearchIndex;
        if (index is not null && index.Revision == revision) return index;

        index = new SearchIndexEntry
        {
            Revision = revision,
            Title = TextNormalizer.Normalize(item.DisplayTitle),
            Target = TextNormalizer.Normalize(item.Target),
            Host = NormalizeHost(item.Target),
            FolderPath = TextNormalizer.Normalize(item.FolderPathText),
            Note = TextNormalizer.Normalize(item.Note),
            Tags = item.Tags.Select(TextNormalizer.Normalize).ToArray(),
        };
        index.FolderSegments = index.FolderPath
            .Split('>', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);

        item.SearchIndex = index;
        return index;
    }

    /// <summary>"www." を落としたホスト名。"github" で www.github.com を先頭一致させるため。</summary>
    private static string NormalizeHost(string? target)
    {
        var host = TargetResolver.TryGetHost(target);
        if (string.IsNullOrEmpty(host)) return string.Empty;
        if (host.StartsWith("www.", StringComparison.OrdinalIgnoreCase)) host = host[4..];
        return TextNormalizer.Normalize(host);
    }
}

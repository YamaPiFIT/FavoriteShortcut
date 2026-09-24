using System.IO;
using System.Text.Json;

namespace FavoriteShortcut.Services;

/// <summary>取り込み元として選べるブラウザのプロファイル。</summary>
public sealed record BookmarkProfile(string BrowserName, string ProfileLabel, string FilePath)
{
    /// <summary>一覧に出す表示名。プロファイルが 1 つだけなら括弧は付けない。</summary>
    public string DisplayName => string.IsNullOrEmpty(ProfileLabel)
        ? BrowserName
        : $"{BrowserName}（{ProfileLabel}）";

    public override string ToString() => DisplayName;
}

/// <summary>お気に入り（ブックマーク）1 項目。Url が null ならフォルダ。</summary>
public sealed class BookmarkNode
{
    public string Name { get; init; } = string.Empty;
    public string? Url { get; init; }
    public DateTime? AddedAt { get; init; }
    public List<BookmarkNode> Children { get; } = new();

    public bool IsFolder => Url is null;

    /// <summary>配下にあるリンクの総数（フォルダは数えない）。</summary>
    public int LinkCount => IsFolder ? Children.Sum(c => c.LinkCount) : 1;

    /// <summary>配下にあるフォルダの総数（自分は数えない）。</summary>
    public int FolderCount => Children.Sum(c => c.IsFolder ? 1 + c.FolderCount : 0);
}

/// <summary>
/// Chromium 系ブラウザ（Edge / Chrome など）のお気に入りを読み取る。
///
/// これらのブラウザは、プロファイルフォルダ内の "Bookmarks" という JSON ファイルに
/// お気に入りを保存している。ファイルを読むだけで、ブラウザ側には一切書き込まない。
/// </summary>
public static class BrowserBookmarkReader
{
    /// <summary>この PC で取り込み可能なプロファイルを探す。</summary>
    public static List<BookmarkProfile> FindProfiles()
    {
        var result = new List<BookmarkProfile>();

        var localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        var roamingAppData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);

        // Edge と Chrome が主目的。他の Chromium 系も同じ形式なので、
        // インストールされていれば同様に選べる。
        var roots = new (string Path, string Name)[]
        {
            (Path.Combine(localAppData, "Microsoft", "Edge", "User Data"), "Microsoft Edge"),
            (Path.Combine(localAppData, "Google", "Chrome", "User Data"), "Google Chrome"),
            (Path.Combine(localAppData, "BraveSoftware", "Brave-Browser", "User Data"), "Brave"),
            (Path.Combine(localAppData, "Vivaldi", "User Data"), "Vivaldi"),
            (Path.Combine(localAppData, "Chromium", "User Data"), "Chromium"),
            (Path.Combine(roamingAppData, "Opera Software", "Opera Stable"), "Opera"),
            (Path.Combine(roamingAppData, "Opera Software", "Opera GX Stable"), "Opera GX"),
        };

        foreach (var (root, browserName) in roots)
        {
            if (!Directory.Exists(root)) continue;

            var friendlyNames = ReadProfileNames(root);

            // Opera のようにプロファイル直下に置く構成
            var direct = Path.Combine(root, "Bookmarks");
            if (File.Exists(direct)) result.Add(new BookmarkProfile(browserName, string.Empty, direct));

            // Chrome / Edge のように Default, Profile 1 ... に分かれる構成
            try
            {
                foreach (var dir in Directory.EnumerateDirectories(root))
                {
                    var leaf = Path.GetFileName(dir);
                    if (!leaf.Equals("Default", StringComparison.OrdinalIgnoreCase) &&
                        !leaf.StartsWith("Profile ", StringComparison.OrdinalIgnoreCase))
                        continue;

                    var file = Path.Combine(dir, "Bookmarks");
                    if (!File.Exists(file)) continue;

                    var label = friendlyNames.GetValueOrDefault(leaf) ?? leaf;
                    result.Add(new BookmarkProfile(browserName, label, file));
                }
            }
            catch (Exception ex)
            {
                AppLog.Warn($"プロファイルを列挙できませんでした: {root}", ex);
            }
        }

        return result;
    }

    /// <summary>
    /// Local State からプロファイルの表示名（「仕事用」など利用者が付けた名前）を読む。
    /// 取れなくてもフォルダ名で代用できるので、失敗は無視する。
    /// </summary>
    private static Dictionary<string, string> ReadProfileNames(string userDataRoot)
    {
        var names = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);

        try
        {
            var localState = Path.Combine(userDataRoot, "Local State");
            if (!File.Exists(localState)) return names;

            using var document = JsonDocument.Parse(ReadFileSafely(localState));
            if (!document.RootElement.TryGetProperty("profile", out var profile)) return names;
            if (!profile.TryGetProperty("info_cache", out var cache)) return names;

            foreach (var entry in cache.EnumerateObject())
            {
                if (entry.Value.TryGetProperty("name", out var name) &&
                    name.GetString() is { Length: > 0 } text)
                {
                    names[entry.Name] = text;
                }
            }
        }
        catch (Exception ex)
        {
            AppLog.Warn($"プロファイル名を読めませんでした: {userDataRoot}", ex);
        }

        return names;
    }

    /// <summary>
    /// Bookmarks ファイルを読み、最上位のフォルダ（ブックマーク バー / その他 など）を返す。
    /// </summary>
    public static List<BookmarkNode> ReadRoots(string bookmarksFilePath)
    {
        var json = ReadFileSafely(bookmarksFilePath);
        using var document = JsonDocument.Parse(json);

        var result = new List<BookmarkNode>();
        if (!document.RootElement.TryGetProperty("roots", out var roots)) return result;

        // bookmark_bar → other → synced の順に並べたい（JSON の並び順は保証されないため）
        var order = new[] { "bookmark_bar", "other", "synced" };
        var seen = new HashSet<string>(StringComparer.Ordinal);

        foreach (var key in order)
        {
            if (!roots.TryGetProperty(key, out var element)) continue;
            seen.Add(key);
            if (Parse(element) is { } node && node.IsFolder) result.Add(node);
        }

        foreach (var property in roots.EnumerateObject())
        {
            if (seen.Contains(property.Name)) continue;
            if (property.Value.ValueKind != JsonValueKind.Object) continue;
            if (Parse(property.Value) is { } node && node.IsFolder) result.Add(node);
        }

        return result;
    }

    private static BookmarkNode? Parse(JsonElement element)
    {
        if (element.ValueKind != JsonValueKind.Object) return null;

        var type = element.TryGetProperty("type", out var t) ? t.GetString() : null;
        var name = element.TryGetProperty("name", out var n) ? n.GetString() ?? string.Empty : string.Empty;
        var addedAt = element.TryGetProperty("date_added", out var d) ? ParseChromeTime(d) : null;

        if (type == "url")
        {
            var url = element.TryGetProperty("url", out var u) ? u.GetString() : null;
            if (string.IsNullOrWhiteSpace(url)) return null;

            // ブックマークレット（javascript:）はショートカットとして開けないので除外する
            if (url.StartsWith("javascript:", StringComparison.OrdinalIgnoreCase)) return null;
            if (url.StartsWith("chrome://", StringComparison.OrdinalIgnoreCase)) return null;
            if (url.StartsWith("edge://", StringComparison.OrdinalIgnoreCase)) return null;

            return new BookmarkNode
            {
                Name = name.Length > 0 ? name : url,
                Url = url,
                AddedAt = addedAt,
            };
        }

        // type が無い場合もフォルダ（roots 直下など）として扱う
        var folder = new BookmarkNode { Name = name, AddedAt = addedAt };

        if (element.TryGetProperty("children", out var children) &&
            children.ValueKind == JsonValueKind.Array)
        {
            foreach (var child in children.EnumerateArray())
                if (Parse(child) is { } parsed) folder.Children.Add(parsed);
        }

        return folder;
    }

    /// <summary>
    /// Chromium の date_added は 1601-01-01 からのマイクロ秒。
    /// 値が壊れていても取り込み自体は続けたいので、失敗したら null を返す。
    /// </summary>
    private static DateTime? ParseChromeTime(JsonElement element)
    {
        try
        {
            var text = element.ValueKind == JsonValueKind.String ? element.GetString() : element.ToString();
            if (!long.TryParse(text, out var microseconds) || microseconds <= 0) return null;

            return DateTime.FromFileTimeUtc(microseconds * 10);
        }
        catch
        {
            return null;
        }
    }

    /// <summary>
    /// ブラウザが起動中でも読めるようにする。
    /// 直接読めない場合だけ一時コピーを経由し、読み終わったら削除する。
    /// </summary>
    private static string ReadFileSafely(string path)
    {
        try
        {
            using var stream = new FileStream(
                path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            using var reader = new StreamReader(stream);
            return reader.ReadToEnd();
        }
        catch (IOException)
        {
            var temp = Path.Combine(Path.GetTempPath(), "fsc-bookmarks-" + Guid.NewGuid().ToString("N"));
            try
            {
                File.Copy(path, temp, overwrite: true);
                return File.ReadAllText(temp);
            }
            finally
            {
                try { if (File.Exists(temp)) File.Delete(temp); } catch { /* 後始末の失敗は無視 */ }
            }
        }
    }
}

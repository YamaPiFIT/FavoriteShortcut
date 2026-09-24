using System.IO;
using Microsoft.Data.Sqlite;

namespace FavoriteShortcut.Services;

/// <summary>
/// ブラウザが保存している favicon のキャッシュから、アイコン画像を取り出す。
///
/// このアプリからブラウザのプロセスを覗くことはできないが、ブラウザは表示したサイトの
/// favicon を自分の SQLite に保存している。そこを読むことで、
/// bot 対策で弾かれるサイト・認証が必要な社内サイト・JS で描画されるサイトなど、
/// HTTP で直接取りに行っても取得できないアイコンを表示できる（オフラインでも動作する）。
///
/// 取り扱いの方針:
///   - 通信は一切行わない。ローカルファイルを読み取り専用で開くだけ
///   - 認証情報は扱わない
///   - 登録済みショートカットの URL でピンポイントに検索する。一覧の走査はしない
///   - 取り出すのは一致したアイコン画像のみ。URL や閲覧履歴は保持しない
///   - ブラウザのファイルには一切書き込まない（一時コピーを読む）
/// </summary>
public sealed class BrowserFaviconCache : IDisposable
{
    /// <summary>
    /// 一時コピーを保持する時間。起動直後にまとめて問い合わせが来るため、
    /// 短時間だけ使い回してコピー回数を抑え、それを過ぎたら確実に削除する。
    /// </summary>
    private static readonly TimeSpan SnapshotLifetime = TimeSpan.FromSeconds(20);

    private readonly object _gate = new();
    private List<Snapshot>? _snapshots;
    private DateTime _snapshotCreatedAt;
    private Timer? _cleanupTimer;

    private sealed record Snapshot(string Path, BrowserKind Kind, string SourceName);

    private enum BrowserKind
    {
        Chromium,
        Firefox,
    }

    /// <summary>見つかったアイコン。Data は PNG / ICO などの画像バイト列。</summary>
    public sealed record Found(byte[] Data, string BrowserName);

    /// <summary>
    /// 指定した URL のアイコンをブラウザのキャッシュから探す。見つからなければ null。
    /// ファイル I/O を伴うので UI スレッドから直接呼ばないこと。
    /// </summary>
    public Found? TryFind(Uri pageUri)
    {
        foreach (var snapshot in GetSnapshots())
        {
            try
            {
                var data = snapshot.Kind == BrowserKind.Chromium
                    ? ReadChromium(snapshot.Path, pageUri)
                    : ReadFirefox(snapshot.Path, pageUri);

                if (data is not null) return new Found(data, snapshot.SourceName);
            }
            catch (Exception ex)
            {
                AppLog.Warn($"ブラウザのアイコンキャッシュを読めませんでした: {snapshot.SourceName}", ex);
            }
        }

        return null;
    }

    // ------------------------------------------------------- 問い合わせ本体

    /// <summary>Chromium 系（Chrome / Edge / Brave / Vivaldi / Opera）の Favicons を読む。</summary>
    public static byte[]? ReadChromium(string databasePath, Uri pageUri)
    {
        using var connection = OpenReadOnly(databasePath);

        using var cmd = connection.CreateCommand();
        cmd.CommandText =
            "SELECT b.image_data, b.width, m.page_url " +
            "FROM icon_mapping m " +
            "JOIN favicon_bitmaps b ON b.icon_id = m.icon_id " +
            "WHERE length(b.image_data) > 0 " +
            "  AND (m.page_url = $exact OR m.page_url LIKE $prefix ESCAPE '\\') " +
            "LIMIT 60";
        BindLookup(cmd, pageUri);

        return PickBest(cmd, pageUri);
    }

    /// <summary>Firefox の favicons.sqlite を読む。</summary>
    public static byte[]? ReadFirefox(string databasePath, Uri pageUri)
    {
        using var connection = OpenReadOnly(databasePath);

        using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText =
                "SELECT i.data, i.width, p.page_url " +
                "FROM moz_icons i " +
                "JOIN moz_icons_to_pages ip ON ip.icon_id = i.id " +
                "JOIN moz_pages_w_icons p ON p.id = ip.page_id " +
                "WHERE length(i.data) > 0 " +
                "  AND (p.page_url = $exact OR p.page_url LIKE $prefix ESCAPE '\\') " +
                "LIMIT 60";
            BindLookup(cmd, pageUri);

            if (PickBest(cmd, pageUri) is { } data) return data;
        }

        // ページとの対応が無くても、サイト直下の favicon.ico が別途保存されていることがある
        using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText =
                "SELECT data, width, icon_url FROM moz_icons " +
                "WHERE root = 1 AND length(data) > 0 AND icon_url LIKE $prefix ESCAPE '\\' " +
                "LIMIT 20";
            cmd.Parameters.AddWithValue("$prefix", LikePrefix(pageUri));

            return PickBest(cmd, pageUri);
        }
    }

    private static void BindLookup(SqliteCommand cmd, Uri pageUri)
    {
        cmd.Parameters.AddWithValue("$exact", pageUri.AbsoluteUri);
        cmd.Parameters.AddWithValue("$prefix", LikePrefix(pageUri));
    }

    /// <summary>
    /// 候補の中から表示に適したものを選ぶ。
    /// 登録 URL と完全一致するものを優先し、次に 64px に近いサイズを選ぶ。
    /// </summary>
    private static byte[]? PickBest(SqliteCommand cmd, Uri pageUri)
    {
        byte[]? best = null;
        var bestScore = int.MaxValue;

        using var reader = cmd.ExecuteReader();
        while (reader.Read())
        {
            if (reader.IsDBNull(0)) continue;

            var data = (byte[])reader[0];
            if (data.Length < 16) continue;

            // WPF で描けない形式（SVG など）は採用しない
            if (IconService.DetectImageExtension(data) is null) continue;

            var width = reader.IsDBNull(1) ? 0 : reader.GetInt32(1);
            var url = reader.IsDBNull(2) ? string.Empty : reader.GetString(2);

            var score = SizeScore(width);
            if (string.Equals(url, pageUri.AbsoluteUri, StringComparison.OrdinalIgnoreCase))
                score -= 10000; // 完全一致は無条件で優先

            if (score >= bestScore) continue;
            bestScore = score;
            best = data;
        }

        return best;

        // 64px に近いほど良い。32px 未満は大きく減点する。
        static int SizeScore(int width) =>
            width >= 32 ? Math.Abs(width - 64) : 1000 + (32 - width);
    }

    /// <summary>LIKE 用に "https://host:port/%" を組み立てる（ワイルドカードはエスケープする）。</summary>
    private static string LikePrefix(Uri pageUri)
    {
        var authority = pageUri.IsDefaultPort ? pageUri.Host : $"{pageUri.Host}:{pageUri.Port}";
        var origin = $"{pageUri.Scheme}://{authority}";

        var escaped = origin
            .Replace("\\", "\\\\", StringComparison.Ordinal)
            .Replace("%", "\\%", StringComparison.Ordinal)
            .Replace("_", "\\_", StringComparison.Ordinal);

        return escaped + "/%";
    }

    private static SqliteConnection OpenReadOnly(string path)
    {
        var csb = new SqliteConnectionStringBuilder
        {
            DataSource = path,
            Mode = SqliteOpenMode.ReadOnly,
        };
        var connection = new SqliteConnection(csb.ToString());
        connection.Open();
        return connection;
    }

    // ------------------------------------------------- プロファイルの探索

    /// <summary>
    /// この PC で参照できるブラウザの名前を返す（設定画面の表示用）。
    /// ファイルの存在を確認するだけで、中身は読まない。
    /// </summary>
    public static List<string> DetectAvailableBrowsers()
    {
        return FindProfileDatabases()
            .Select(x => x.Name.Split(' ')[0])
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    private List<Snapshot> GetSnapshots()
    {
        lock (_gate)
        {
            if (_snapshots is not null && DateTime.UtcNow - _snapshotCreatedAt < SnapshotLifetime)
                return _snapshots;

            DeleteSnapshots();

            _snapshots = new List<Snapshot>();
            _snapshotCreatedAt = DateTime.UtcNow;

            foreach (var (path, kind, name) in FindProfileDatabases())
            {
                var copy = TryCopyForReading(path);
                if (copy is not null) _snapshots.Add(new Snapshot(copy, kind, name));
            }

            // 一時コピーが残り続けないよう、一定時間で必ず消す
            _cleanupTimer?.Dispose();
            _cleanupTimer = new Timer(_ => { lock (_gate) DeleteSnapshots(); }, null,
                SnapshotLifetime, Timeout.InfiniteTimeSpan);

            return _snapshots;
        }
    }

    /// <summary>この PC にあるブラウザのアイコンキャッシュを列挙する。</summary>
    private static List<(string Path, BrowserKind Kind, string Name)> FindProfileDatabases()
    {
        var result = new List<(string, BrowserKind, string)>();

        var localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        var roamingAppData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);

        // Chromium 系はいずれも同じ Favicons スキーマを使う
        var chromiumRoots = new (string Path, string Name)[]
        {
            (Path.Combine(localAppData, "Google", "Chrome", "User Data"), "Chrome"),
            (Path.Combine(localAppData, "Microsoft", "Edge", "User Data"), "Edge"),
            (Path.Combine(localAppData, "BraveSoftware", "Brave-Browser", "User Data"), "Brave"),
            (Path.Combine(localAppData, "Vivaldi", "User Data"), "Vivaldi"),
            (Path.Combine(localAppData, "Chromium", "User Data"), "Chromium"),
            (Path.Combine(roamingAppData, "Opera Software", "Opera Stable"), "Opera"),
            (Path.Combine(roamingAppData, "Opera Software", "Opera GX Stable"), "Opera GX"),
        };

        foreach (var (root, name) in chromiumRoots)
        {
            if (!Directory.Exists(root)) continue;

            // Opera のようにプロファイル直下に置く構成と、Chrome のように
            // Default / Profile 1 ... に分かれる構成の両方に対応する
            AddIfExists(result, Path.Combine(root, "Favicons"), BrowserKind.Chromium, name);

            try
            {
                foreach (var dir in Directory.EnumerateDirectories(root))
                {
                    var leaf = Path.GetFileName(dir);
                    if (!leaf.Equals("Default", StringComparison.OrdinalIgnoreCase) &&
                        !leaf.StartsWith("Profile ", StringComparison.OrdinalIgnoreCase))
                        continue;

                    AddIfExists(result, Path.Combine(dir, "Favicons"), BrowserKind.Chromium, $"{name} ({leaf})");
                }
            }
            catch (Exception ex)
            {
                AppLog.Warn($"ブラウザのプロファイルを列挙できませんでした: {root}", ex);
            }
        }

        var firefoxProfiles = Path.Combine(roamingAppData, "Mozilla", "Firefox", "Profiles");
        if (Directory.Exists(firefoxProfiles))
        {
            try
            {
                foreach (var dir in Directory.EnumerateDirectories(firefoxProfiles))
                    AddIfExists(result, Path.Combine(dir, "favicons.sqlite"), BrowserKind.Firefox,
                        $"Firefox ({Path.GetFileName(dir)})");
            }
            catch (Exception ex)
            {
                AppLog.Warn("Firefox のプロファイルを列挙できませんでした。", ex);
            }
        }

        // 最近使われたブラウザから先に見る
        return result
            .OrderByDescending(x => SafeLastWriteTime(x.Item1))
            .ToList();
    }

    private static void AddIfExists(
        List<(string, BrowserKind, string)> list, string path, BrowserKind kind, string name)
    {
        if (File.Exists(path)) list.Add((path, kind, name));
    }

    private static DateTime SafeLastWriteTime(string path)
    {
        try { return File.GetLastWriteTimeUtc(path); }
        catch { return DateTime.MinValue; }
    }

    /// <summary>
    /// ブラウザ起動中はファイルが掴まれていることがあるため、一時フォルダへコピーしてから読む。
    /// コピーは <see cref="SnapshotLifetime"/> 経過後に必ず削除する。
    /// </summary>
    private static string? TryCopyForReading(string path)
    {
        try
        {
            var directory = Path.Combine(Path.GetTempPath(), "fsc-icons-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(directory);

            var destination = Path.Combine(directory, Path.GetFileName(path));
            File.Copy(path, destination, overwrite: true);

            // WAL / SHM があれば一緒に持ってこないと最新の内容を読めない
            foreach (var suffix in new[] { "-wal", "-shm" })
            {
                var extra = path + suffix;
                if (File.Exists(extra)) File.Copy(extra, destination + suffix, overwrite: true);
            }

            return destination;
        }
        catch (Exception ex)
        {
            AppLog.Warn($"アイコンキャッシュをコピーできませんでした: {path}", ex);
            return null;
        }
    }

    private void DeleteSnapshots()
    {
        if (_snapshots is null) return;

        foreach (var snapshot in _snapshots)
        {
            try
            {
                SqliteConnection.ClearAllPools();
                var directory = Path.GetDirectoryName(snapshot.Path);
                if (directory is not null && Directory.Exists(directory))
                    Directory.Delete(directory, recursive: true);
            }
            catch (Exception ex)
            {
                AppLog.Warn($"一時コピーを削除できませんでした: {snapshot.Path}", ex);
            }
        }

        _snapshots = null;
    }

    public void Dispose()
    {
        lock (_gate)
        {
            _cleanupTimer?.Dispose();
            _cleanupTimer = null;
            DeleteSnapshots();
        }
    }
}

using System.Collections.Concurrent;
using System.IO;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

/// <summary>
/// ショートカットのアイコン解決（§11, §22, §34）。
///
///   Web        : サイトから直接取得 → ブラウザのアイコンキャッシュ の順に試し、
///                icons/ にキャッシュする。取れなければ既定アイコン。
///   フォルダ   : Windows のフォルダアイコン
///   ファイル   : 関連付けられたアプリのアイコン
///   カスタム   : ユーザーが指定した画像を icons/ にコピー
///
/// DB にはバイナリではなく icons/ 配下の相対ファイル名だけを保存する。
/// キャッシュ済みアイコンはオフラインでも表示できる。
/// </summary>
public sealed class IconService : IDisposable
{
    private static readonly HttpClient Http = CreateHttpClient();

    private static readonly Regex LinkTagRegex = new(
        "<link\\b[^>]*>", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex AttrRegex = new(
        "(?<name>[a-zA-Z\\-]+)\\s*=\\s*(\"(?<v1>[^\"]*)\"|'(?<v2>[^']*)'|(?<v3>[^\\s>]+))",
        RegexOptions.Compiled);

    private readonly AppStore _store;
    private readonly SettingsService _settings;

    /// <summary>ファイル名 -> 読み込み済み画像。</summary>
    private readonly ConcurrentDictionary<string, ImageSource> _memoryCache = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>取得中/取得失敗したホスト。短時間に何度も取りに行かないようにする。</summary>
    private readonly ConcurrentDictionary<string, byte> _inFlight = new(StringComparer.OrdinalIgnoreCase);
    private readonly ConcurrentDictionary<string, byte> _failed = new(StringComparer.OrdinalIgnoreCase);

    private readonly SemaphoreSlim _throttle = new(4, 4);
    private readonly BrowserFaviconCache _browserCache = new();

    public IconService(AppStore store, SettingsService settings)
    {
        _store = store;
        _settings = settings;
    }

    public void Dispose() => _browserCache.Dispose();

    private static HttpClient CreateHttpClient()
    {
        var handler = new HttpClientHandler
        {
            AllowAutoRedirect = true,
            MaxAutomaticRedirections = 5,
            AutomaticDecompression = System.Net.DecompressionMethods.All,
        };
        var client = new HttpClient(handler)
        {
            Timeout = TimeSpan.FromSeconds(8),
            MaxResponseContentBufferSize = 4 * 1024 * 1024,
        };
        client.DefaultRequestHeaders.UserAgent.ParseAdd(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) FavoriteShortcut/1.0");
        client.DefaultRequestHeaders.AcceptLanguage.ParseAdd("ja,en;q=0.8");
        return client;
    }

    public static string FullPath(string relativeFileName) =>
        Path.Combine(AppPaths.IconDirectory, relativeFileName);

    // --------------------------------------------------------------- resolve

    /// <summary>
    /// 表示用アイコンを item.Icon に設定する。Web で favicon 未取得なら裏で取得を始める。
    /// UI スレッドから呼ぶこと。
    /// </summary>
    public void Attach(ShortcutItem item)
    {
        item.Icon = ResolveSync(item);

        if (item.TargetType == TargetType.Web && string.IsNullOrEmpty(item.IconPath))
            _ = EnsureFaviconAsync(item, force: false);
    }

    /// <summary>その場で決まるアイコン（保存済みファイル / シェル / 既定）を返す。</summary>
    public ImageSource ResolveSync(ShortcutItem item)
    {
        if (!string.IsNullOrEmpty(item.IconPath))
        {
            var loaded = LoadFromCache(item.IconPath);
            if (loaded is not null) return loaded;
        }

        var path = TargetResolver.Expand(item.Target ?? string.Empty);
        return item.TargetType switch
        {
            TargetType.Folder => ShellIconProvider.GetFolderIcon(path) ?? DefaultIcons.Folder,
            TargetType.File => ShellIconProvider.GetFileIcon(path) ?? DefaultIcons.File,
            TargetType.Application => ShellIconProvider.GetFileIcon(path) ?? DefaultIcons.For(TargetType.Application),
            TargetType.Web => DefaultIcons.Web,
            _ => DefaultIcons.Unknown,
        };
    }

    private ImageSource? LoadFromCache(string relativeFileName)
    {
        if (_memoryCache.TryGetValue(relativeFileName, out var cached)) return cached;

        var full = FullPath(relativeFileName);
        if (!File.Exists(full)) return null;

        var image = LoadImageFile(full);
        if (image is not null) _memoryCache[relativeFileName] = image;
        return image;
    }

    /// <summary>ico / png / jpg / gif / bmp を読み込む。ico は表示に適したサイズのフレームを選ぶ。</summary>
    public static ImageSource? LoadImageFile(string path)
    {
        try
        {
            using var stream = File.OpenRead(path);
            var decoder = BitmapDecoder.Create(
                stream, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
            if (decoder.Frames.Count == 0) return null;

            BitmapFrame? best = null;
            foreach (var frame in decoder.Frames)
            {
                if (best is null) { best = frame; continue; }
                // 64px に一番近く、かつなるべく 32px 以上のフレームを選ぶ
                var bestScore = Score(best.PixelWidth);
                var score = Score(frame.PixelWidth);
                if (score < bestScore) best = frame;
            }

            if (best is null) return null;
            if (best.CanFreeze) best.Freeze();
            return best;

            static int Score(int width) => width >= 32 ? Math.Abs(width - 64) : 1000 + (32 - width);
        }
        catch (Exception ex)
        {
            AppLog.Warn($"画像を読み込めませんでした: {path}", ex);
            return null;
        }
    }

    public void ClearMemoryCache()
    {
        _memoryCache.Clear();
        _failed.Clear();
        ShellIconProvider.ClearCache();
    }

    // --------------------------------------------------------------- favicon

    /// <summary>
    /// Web ショートカットの favicon を取得してキャッシュし、item に反映する。
    /// 取得できなければ何もしない（既定アイコンのまま）。
    /// </summary>
    public async Task EnsureFaviconAsync(ShortcutItem item, bool force)
    {
        if (item.TargetType != TargetType.Web) return;

        var pageUri = TargetResolver.TryGetUri(item.Target);
        if (pageUri is null) return;
        if (pageUri.Scheme != Uri.UriSchemeHttp && pageUri.Scheme != Uri.UriSchemeHttps) return;

        // http と https、ポート違いは別サイトなのでオリジン単位でキャッシュする
        var origin = TargetResolver.TryGetOrigin(item.Target);
        if (string.IsNullOrEmpty(origin)) return;

        var fileBase = "fav_" + Hash(origin);

        if (!force)
        {
            // すでにキャッシュがあればそれを使う
            var existing = FindCachedFavicon(fileBase);
            if (existing is not null)
            {
                ApplyIcon(item, existing);
                return;
            }
            if (_failed.ContainsKey(origin)) return;
        }

        if (!_inFlight.TryAdd(origin, 0)) return;

        try
        {
            await _throttle.WaitAsync().ConfigureAwait(false);
            try
            {
                // サイトから直接取得したものを優先し、ブラウザのキャッシュは補助に回す。
                //
                // ブラウザはダークモード用のアイコン（白抜きなど）を保存していることがあり、
                // それを採用すると明るい背景で見えなくなる。サイトが配信している既定の
                // favicon のほうが確実なので、取得できるならそちらを使う。
                // ブラウザのキャッシュは、bot 対策や認証で直接取得できないサイト
                // （社内サイトなど）を救うための経路。
                var saved = await DownloadFaviconAsync(pageUri, fileBase).ConfigureAwait(false)
                            ?? TryBrowserCache(pageUri, fileBase);

                if (saved is null)
                {
                    _failed[origin] = 0;
                    return;
                }

                var app = System.Windows.Application.Current;
                if (app is null) return; // 終了処理中

                await app.Dispatcher.InvokeAsync(() =>
                {
                    _memoryCache.TryRemove(saved, out _);
                    ApplyIcon(item, saved);

                    // 同じオリジンの他のショートカットにも反映する
                    foreach (var other in _store.Shortcuts)
                    {
                        if (ReferenceEquals(other, item)) continue;
                        if (other.TargetType != TargetType.Web) continue;
                        if (!string.IsNullOrEmpty(other.IconPath) && !other.IconPath.StartsWith("fav_", StringComparison.Ordinal))
                            continue;
                        if (!string.Equals(TargetResolver.TryGetOrigin(other.Target), origin, StringComparison.OrdinalIgnoreCase))
                            continue;
                        ApplyIcon(other, saved);
                    }
                });
            }
            finally
            {
                _throttle.Release();
            }
        }
        catch (Exception ex)
        {
            AppLog.Warn($"favicon の取得に失敗: {origin}", ex);
            _failed[origin] = 0;
        }
        finally
        {
            _inFlight.TryRemove(origin, out _);
        }
    }

    /// <summary>ブラウザのアイコンキャッシュから取り出して icons/ に保存する。</summary>
    private string? TryBrowserCache(Uri pageUri, string fileBase)
    {
        if (!_settings.Current.UseBrowserIconCache) return null;

        try
        {
            var found = _browserCache.TryFind(pageUri);
            if (found is null) return null;

            // ブラウザはダークモード用の白いアイコンを保存していることがある。
            // 現在のテーマの背景に埋もれて「アイコンが無い」ように見えるものは採用しない。
            if (IsInvisibleOnCurrentTheme(found.Data))
            {
                AppLog.Info($"背景に埋もれるアイコンのため採用しませんでした: {pageUri.Host}");
                return null;
            }

            var saved = SaveIconBytes(found.Data, fileBase);
            if (saved is not null)
                AppLog.Info($"{found.BrowserName} のアイコンキャッシュから取得しました: {pageUri.Host}");

            return saved;
        }
        catch (Exception ex)
        {
            AppLog.Warn($"ブラウザのアイコンキャッシュから取得できませんでした: {pageUri.Host}", ex);
            return null;
        }
    }

    /// <summary>
    /// 現在のテーマの背景に埋もれて見えないアイコンかどうか。
    ///
    /// ブラウザはサイトのダークモード用アイコン（白一色の抜き文字など）を
    /// 保存していることがあり、そのまま使うと明るい背景では何も見えなくなる。
    /// そうしたものは採用せず、既定アイコンを出したほうが分かりやすい。
    /// </summary>
    private bool IsInvisibleOnCurrentTheme(byte[] data)
    {
        try
        {
            var dark = _settings.Current.Theme == AppTheme.Dark;

            using var stream = new MemoryStream(data);
            var decoder = BitmapDecoder.Create(
                stream, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
            if (decoder.Frames.Count == 0) return false;

            var frame = decoder.Frames[0];
            var converted = new FormatConvertedBitmap(frame, PixelFormats.Bgra32, null, 0);

            var width = converted.PixelWidth;
            var height = converted.PixelHeight;
            if (width <= 0 || height <= 0) return false;

            var stride = width * 4;
            var pixels = new byte[stride * height];
            converted.CopyPixels(pixels, stride, 0);

            var visible = 0;
            var matchesBackground = 0;

            for (var i = 0; i < pixels.Length; i += 4)
            {
                var alpha = pixels[i + 3];
                if (alpha < 40) continue; // ほぼ透明な画素は無視

                visible++;

                // 輝度（おおよその知覚値）
                var luminance = (0.299 * pixels[i + 2] + 0.587 * pixels[i + 1] + 0.114 * pixels[i]) / 255.0;
                if (dark ? luminance < 0.12 : luminance > 0.90) matchesBackground++;
            }

            // 不透明な画素がほとんど無い、または残り全部が背景と同化している
            if (visible < width * height * 0.02) return true;
            return matchesBackground >= visible * 0.97;
        }
        catch (Exception ex)
        {
            AppLog.Warn("アイコンの明るさを判定できませんでした。", ex);
            return false;
        }
    }

    /// <summary>
    /// 起動したショートカットのアイコンを、少し待ってから取り直す。
    ///
    /// ブラウザでサイトを開くと favicon がブラウザ側にキャッシュされるため、
    /// 「一度開けばアイコンが付く」という動きになる（§34 の取得できない場合の補完）。
    /// ブラウザが書き込むまでに間があるので、時間をおいて 2 回試す。
    /// </summary>
    public void ScheduleRecheckAfterLaunch(ShortcutItem item)
    {
        if (item.TargetType != TargetType.Web) return;
        if (!string.IsNullOrEmpty(item.IconPath)) return;
        if (!_settings.Current.UseBrowserIconCache) return;

        var origin = TargetResolver.TryGetOrigin(item.Target);
        if (string.IsNullOrEmpty(origin)) return;

        // 一度失敗していても、ブラウザで開いた後なら取れる可能性があるので再挑戦させる
        _failed.TryRemove(origin, out _);

        _ = Task.Run(async () =>
        {
            foreach (var delay in new[] { 5, 20 })
            {
                await Task.Delay(TimeSpan.FromSeconds(delay)).ConfigureAwait(false);
                if (!string.IsNullOrEmpty(item.IconPath)) return;
                if (System.Windows.Application.Current is null) return;

                _failed.TryRemove(origin, out _);
                await EnsureFaviconAsync(item, force: false).ConfigureAwait(false);
            }
        });
    }

    private void ApplyIcon(ShortcutItem item, string relativeFileName)
    {
        try { _store.UpdateIconPath(item, relativeFileName); }
        catch (Exception ex) { AppLog.Warn("アイコンパスの保存に失敗しました。", ex); }

        item.Icon = LoadFromCache(relativeFileName) ?? DefaultIcons.Web;
    }

    /// <summary>
    /// 画像バイト列を icons/ に保存し、相対ファイル名を返す。
    /// 一時ファイル経由で置き換えるので、途中で失敗しても壊れたファイルが残らない。
    /// </summary>
    private static string? SaveIconBytes(byte[] bytes, string fileBase)
    {
        var ext = DetectImageExtension(bytes);
        if (ext is null) return null; // SVG など WPF で描けない形式は諦める

        var fileName = fileBase + ext;
        var full = FullPath(fileName);

        Directory.CreateDirectory(AppPaths.IconDirectory);

        var temp = full + ".tmp";
        File.WriteAllBytes(temp, bytes);
        if (LoadImageFile(temp) is null)
        {
            File.Delete(temp);
            return null;
        }

        foreach (var old in Directory.EnumerateFiles(AppPaths.IconDirectory, fileBase + ".*"))
            if (!old.EndsWith(".tmp", StringComparison.Ordinal)) File.Delete(old);

        File.Move(temp, full, overwrite: true);
        return fileName;
    }

    private static string? FindCachedFavicon(string fileBase)
    {
        foreach (var ext in new[] { ".png", ".ico", ".jpg", ".gif", ".bmp" })
        {
            var name = fileBase + ext;
            if (File.Exists(FullPath(name))) return name;
        }
        return null;
    }

    private async Task<string?> DownloadFaviconAsync(Uri pageUri, string fileBase)
    {
        foreach (var candidate in await CollectCandidateUrlsAsync(pageUri).ConfigureAwait(false))
        {
            var bytes = await TryDownloadAsync(candidate).ConfigureAwait(false);
            if (bytes is null || bytes.Length < 16) continue;

            try
            {
                if (SaveIconBytes(bytes, fileBase) is { } fileName) return fileName;
            }
            catch (Exception ex)
            {
                AppLog.Warn($"favicon の保存に失敗: {pageUri}", ex);
            }
        }

        return null;
    }

    /// <summary>
    /// favicon の取得先候補を、登録された URL を起点に組み立てる。
    ///
    /// ホスト名だけから "https://host/" を作り直すと、http のみのサイト・
    /// 非標準ポート・サブパスに置かれた社内システムを軒並み取りこぼすため、
    /// 登録された URL のスキーム・ポート・パスをそのまま尊重する。
    /// </summary>
    private async Task<List<string>> CollectCandidateUrlsAsync(Uri pageUri)
    {
        var candidates = new List<string>();
        var origin = pageUri.IsDefaultPort
            ? $"{pageUri.Scheme}://{pageUri.Host}"
            : $"{pageUri.Scheme}://{pageUri.Host}:{pageUri.Port}";

        // 1. 登録された URL の HTML から <link rel="icon"> を探す
        try
        {
            using var response = await Http.GetAsync(pageUri, HttpCompletionOption.ResponseHeadersRead)
                .ConfigureAwait(false);
            if (response.IsSuccessStatusCode)
            {
                var baseUri = response.RequestMessage?.RequestUri ?? pageUri;
                var html = await ReadLimitedStringAsync(response, 256 * 1024).ConfigureAwait(false);
                candidates.AddRange(ExtractIconUrls(html, baseUri));
            }
        }
        catch (Exception ex)
        {
            AppLog.Warn($"HTML の取得に失敗: {pageUri}", ex);
        }

        // 2. サブパスに置かれたアプリ（例 https://intranet/kintai/）の直下
        var directory = GetDirectoryUrl(pageUri);
        if (directory is not null && !string.Equals(directory, origin + "/", StringComparison.OrdinalIgnoreCase))
        {
            candidates.Add(directory + "favicon.ico");
            candidates.Add(directory + "favicon.png");
        }

        // 3. サイトのルート（定番の場所）
        candidates.Add($"{origin}/favicon.ico");
        candidates.Add($"{origin}/favicon.png");

        // 外部のアイコン取得サービスには問い合わせない。
        // 取得できないサイトはブラウザのキャッシュ側で拾えるため、
        // 利用者が登録した相手以外へ通信しない作りにしている。

        return candidates.Distinct(StringComparer.OrdinalIgnoreCase).ToList();
    }

    /// <summary>"https://host/app/page.aspx" → "https://host/app/"</summary>
    private static string? GetDirectoryUrl(Uri uri)
    {
        var path = uri.AbsolutePath;
        if (path.Length == 0) return null;

        var lastSlash = path.LastIndexOf('/');
        if (lastSlash < 0) return null;

        var authority = uri.IsDefaultPort ? uri.Host : $"{uri.Host}:{uri.Port}";
        return $"{uri.Scheme}://{authority}{path[..(lastSlash + 1)]}";
    }

    internal static List<string> ExtractIconUrls(string html, Uri baseUri)
    {
        var found = new List<(int Priority, int Size, string Url)>();

        foreach (Match tag in LinkTagRegex.Matches(html))
        {
            string? rel = null, href = null, sizes = null;
            foreach (Match attr in AttrRegex.Matches(tag.Value))
            {
                var name = attr.Groups["name"].Value.ToLowerInvariant();
                var value = attr.Groups["v1"].Success ? attr.Groups["v1"].Value
                    : attr.Groups["v2"].Success ? attr.Groups["v2"].Value
                    : attr.Groups["v3"].Value;
                switch (name)
                {
                    case "rel": rel = value.ToLowerInvariant(); break;
                    case "href": href = value; break;
                    case "sizes": sizes = value; break;
                }
            }

            if (rel is null || string.IsNullOrWhiteSpace(href)) continue;

            var priority = rel switch
            {
                var r when r.Contains("apple-touch-icon") => 0,
                var r when r.Contains("shortcut icon") => 1,
                var r when r.Contains("icon") => 1,
                _ => -1,
            };
            if (priority < 0) continue;

            var size = 0;
            if (sizes is not null)
            {
                var m = Regex.Match(sizes, @"(\d+)\s*[xX]\s*(\d+)");
                if (m.Success) int.TryParse(m.Groups[1].Value, out size);
            }

            if (Uri.TryCreate(baseUri, href.Trim(), out var abs) &&
                (abs.Scheme == Uri.UriSchemeHttp || abs.Scheme == Uri.UriSchemeHttps))
            {
                found.Add((priority, size, abs.ToString()));
            }
        }

        return found
            .OrderBy(f => f.Priority)
            .ThenByDescending(f => f.Size)
            .Select(f => f.Url)
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .Take(5)
            .ToList();
    }

    private static async Task<byte[]?> TryDownloadAsync(string url)
    {
        try
        {
            using var response = await Http.GetAsync(url, HttpCompletionOption.ResponseHeadersRead).ConfigureAwait(false);
            if (!response.IsSuccessStatusCode) return null;
            if (response.Content.Headers.ContentLength > 2 * 1024 * 1024) return null;
            return await response.Content.ReadAsByteArrayAsync().ConfigureAwait(false);
        }
        catch
        {
            return null;
        }
    }

    private static async Task<string> ReadLimitedStringAsync(HttpResponseMessage response, int maxBytes)
    {
        await using var stream = await response.Content.ReadAsStreamAsync().ConfigureAwait(false);
        var buffer = new byte[maxBytes];
        var total = 0;
        while (total < maxBytes)
        {
            var read = await stream.ReadAsync(buffer.AsMemory(total, maxBytes - total)).ConfigureAwait(false);
            if (read == 0) break;
            total += read;
        }
        return Encoding.UTF8.GetString(buffer, 0, total);
    }

    /// <summary>マジックナンバーから画像形式を判定する（拡張子は当てにしない）。</summary>
    internal static string? DetectImageExtension(byte[] bytes)
    {
        if (bytes.Length >= 8 && bytes[0] == 0x89 && bytes[1] == 0x50 && bytes[2] == 0x4E && bytes[3] == 0x47)
            return ".png";
        if (bytes.Length >= 4 && bytes[0] == 0x00 && bytes[1] == 0x00 && (bytes[2] == 0x01 || bytes[2] == 0x02) && bytes[3] == 0x00)
            return ".ico";
        if (bytes.Length >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF)
            return ".jpg";
        if (bytes.Length >= 6 && bytes[0] == 'G' && bytes[1] == 'I' && bytes[2] == 'F')
            return ".gif";
        if (bytes.Length >= 2 && bytes[0] == 'B' && bytes[1] == 'M')
            return ".bmp";
        return null;
    }

    private static string Hash(string value)
    {
        var bytes = SHA1.HashData(Encoding.UTF8.GetBytes(value.ToLowerInvariant()));
        return Convert.ToHexString(bytes)[..16].ToLowerInvariant();
    }

    // ---------------------------------------------------------- custom icons

    /// <summary>ユーザーが選んだ画像を icons/ にコピーして、相対ファイル名を返す。</summary>
    public string? ImportCustomIcon(string sourcePath)
    {
        try
        {
            var bytes = File.ReadAllBytes(sourcePath);
            var ext = DetectImageExtension(bytes) ?? Path.GetExtension(sourcePath).ToLowerInvariant();
            if (string.IsNullOrEmpty(ext)) ext = ".png";

            var fileName = "custom_" + Guid.NewGuid().ToString("N") + ext;
            var full = FullPath(fileName);
            Directory.CreateDirectory(AppPaths.IconDirectory);
            File.WriteAllBytes(full, bytes);

            if (LoadImageFile(full) is null)
            {
                File.Delete(full);
                return null;
            }

            return fileName;
        }
        catch (Exception ex)
        {
            AppLog.Warn($"アイコンの取り込みに失敗: {sourcePath}", ex);
            return null;
        }
    }

    /// <summary>カスタムアイコンを削除する（favicon は他のショートカットと共有なので消さない）。</summary>
    public void DeleteCustomIcon(string? relativeFileName)
    {
        if (string.IsNullOrEmpty(relativeFileName)) return;
        if (!relativeFileName.StartsWith("custom_", StringComparison.Ordinal)) return;

        try
        {
            _memoryCache.TryRemove(relativeFileName, out _);
            var full = FullPath(relativeFileName);
            if (File.Exists(full)) File.Delete(full);
        }
        catch (Exception ex)
        {
            AppLog.Warn($"アイコンの削除に失敗: {relativeFileName}", ex);
        }
    }

    /// <summary>どのショートカットからも参照されていないアイコンファイルを削除する。</summary>
    public int CleanUpUnusedIcons()
    {
        var used = _store.Shortcuts
            .Select(s => s.IconPath)
            .Where(p => !string.IsNullOrEmpty(p))
            .ToHashSet(StringComparer.OrdinalIgnoreCase)!;

        var removed = 0;
        try
        {
            foreach (var file in Directory.EnumerateFiles(AppPaths.IconDirectory))
            {
                var name = Path.GetFileName(file);
                if (used.Contains(name)) continue;
                File.Delete(file);
                removed++;
            }
        }
        catch (Exception ex)
        {
            AppLog.Warn("未使用アイコンの整理に失敗しました。", ex);
        }

        _memoryCache.Clear();
        return removed;
    }
}

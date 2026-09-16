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
///   Web        : サイトの favicon を取得して icons/ にキャッシュ。取れなければ既定アイコン。
///   フォルダ   : Windows のフォルダアイコン
///   ファイル   : 関連付けられたアプリのアイコン
///   カスタム   : ユーザーが指定した画像を icons/ にコピー
///
/// DB にはバイナリではなく icons/ 配下の相対ファイル名だけを保存する。
/// キャッシュ済みアイコンはオフラインでも表示できる。
/// </summary>
public sealed class IconService
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

    public IconService(AppStore store, SettingsService settings)
    {
        _store = store;
        _settings = settings;
    }

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

        var host = TargetResolver.TryGetHost(item.Target);
        if (string.IsNullOrEmpty(host)) return;

        var fileBase = "fav_" + Hash(host);

        if (!force)
        {
            // すでにキャッシュがあればそれを使う
            var existing = FindCachedFavicon(fileBase);
            if (existing is not null)
            {
                ApplyIcon(item, existing);
                return;
            }
            if (_failed.ContainsKey(host)) return;
        }

        if (!_inFlight.TryAdd(host, 0)) return;

        try
        {
            await _throttle.WaitAsync().ConfigureAwait(false);
            try
            {
                var saved = await DownloadFaviconAsync(host, fileBase).ConfigureAwait(false);
                if (saved is null)
                {
                    _failed[host] = 0;
                    return;
                }

                var app = System.Windows.Application.Current;
                if (app is null) return; // 終了処理中

                await app.Dispatcher.InvokeAsync(() =>
                {
                    _memoryCache.TryRemove(saved, out _);
                    ApplyIcon(item, saved);

                    // 同じホストの他のショートカットにも反映する
                    foreach (var other in _store.Shortcuts)
                    {
                        if (ReferenceEquals(other, item)) continue;
                        if (other.TargetType != TargetType.Web) continue;
                        if (!string.IsNullOrEmpty(other.IconPath) && !other.IconPath.StartsWith("fav_", StringComparison.Ordinal))
                            continue;
                        if (!string.Equals(TargetResolver.TryGetHost(other.Target), host, StringComparison.OrdinalIgnoreCase))
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
            AppLog.Warn($"favicon の取得に失敗: {host}", ex);
            _failed[host] = 0;
        }
        finally
        {
            _inFlight.TryRemove(host, out _);
        }
    }

    private void ApplyIcon(ShortcutItem item, string relativeFileName)
    {
        try { _store.UpdateIconPath(item, relativeFileName); }
        catch (Exception ex) { AppLog.Warn("アイコンパスの保存に失敗しました。", ex); }

        item.Icon = LoadFromCache(relativeFileName) ?? DefaultIcons.Web;
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

    private async Task<string?> DownloadFaviconAsync(string host, string fileBase)
    {
        foreach (var candidate in await CollectCandidateUrlsAsync(host).ConfigureAwait(false))
        {
            var bytes = await TryDownloadAsync(candidate).ConfigureAwait(false);
            if (bytes is null || bytes.Length < 16) continue;

            var ext = DetectImageExtension(bytes);
            if (ext is null) continue; // SVG など WPF で描けない形式は諦める

            var fileName = fileBase + ext;
            var full = FullPath(fileName);
            try
            {
                Directory.CreateDirectory(AppPaths.IconDirectory);
                // 一時ファイル経由で置き換え、途中で失敗しても壊れたファイルが残らないようにする
                var temp = full + ".tmp";
                await File.WriteAllBytesAsync(temp, bytes).ConfigureAwait(false);
                if (LoadImageFile(temp) is null) { File.Delete(temp); continue; }

                foreach (var old in Directory.EnumerateFiles(AppPaths.IconDirectory, fileBase + ".*"))
                    if (!old.EndsWith(".tmp", StringComparison.Ordinal)) File.Delete(old);

                File.Move(temp, full, overwrite: true);
                return fileName;
            }
            catch (Exception ex)
            {
                AppLog.Warn($"favicon の保存に失敗: {host}", ex);
            }
        }

        return null;
    }

    private async Task<List<string>> CollectCandidateUrlsAsync(string host)
    {
        var candidates = new List<string>();

        // 1. HTML の <link rel="icon"> を見る
        try
        {
            using var response = await Http.GetAsync($"https://{host}/", HttpCompletionOption.ResponseHeadersRead)
                .ConfigureAwait(false);
            if (response.IsSuccessStatusCode)
            {
                var baseUri = response.RequestMessage?.RequestUri ?? new Uri($"https://{host}/");
                var html = await ReadLimitedStringAsync(response, 256 * 1024).ConfigureAwait(false);
                candidates.AddRange(ExtractIconUrls(html, baseUri));
            }
        }
        catch (Exception ex)
        {
            AppLog.Warn($"HTML の取得に失敗: {host}", ex);
        }

        // 2. 定番の場所
        candidates.Add($"https://{host}/favicon.ico");
        candidates.Add($"https://{host}/favicon.png");

        // 3. 設定で許可されている場合のみ外部サービス（ドメイン名が外部に送信されるため既定はオフ）
        if (_settings.Current.UseFaviconFallbackService)
            candidates.Add($"https://www.google.com/s2/favicons?sz=64&domain={Uri.EscapeDataString(host)}");

        return candidates.Distinct(StringComparer.OrdinalIgnoreCase).ToList();
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

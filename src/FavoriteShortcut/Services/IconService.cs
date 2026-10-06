using System.Collections.Concurrent;
using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Services;

/// <summary>
/// ショートカットのアイコン解決（§11, §22, §34）。
///
///   Web        : ブラウザが保存しているアイコン（ブラウザで開いたことのあるサイト）を
///                icons/ にコピーして使う。取れなければ既定アイコン。
///                アプリからサイトへの通信は行わない。
///   フォルダ   : Windows のフォルダアイコン
///   ファイル   : 関連付けられたアプリのアイコン
///   カスタム   : ユーザーが指定した画像を icons/ にコピー
///
/// DB にはバイナリではなく icons/ 配下の相対ファイル名だけを保存する。
/// キャッシュ済みアイコンはオフラインでも表示できる。
///
/// 画面の表示を待たせないよう、ブラウザのファイルの読み取りと、
/// 共有フォルダ（ネットワーク上）のフォルダ・ファイルのアイコン取得は裏で行う。
/// </summary>
public sealed class IconService : IDisposable
{
    private readonly AppStore _store;
    private readonly SettingsService _settings;

    /// <summary>ファイル名 -> 読み込み済み画像。</summary>
    private readonly ConcurrentDictionary<string, ImageSource> _memoryCache = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>
    /// ブラウザのアイコンを探している / 見つからなかったサイト。
    /// 見つからなかったサイトは、アプリから開くか「アイコンを再取得」するまで探し直さない。
    /// </summary>
    private readonly ConcurrentDictionary<string, byte> _inFlight = new(StringComparer.OrdinalIgnoreCase);
    private readonly ConcurrentDictionary<string, byte> _failed = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>
    /// ブラウザのファイルは 1 件ずつ読む。一時コピーを使い回すので、同時に読んでも速くはならず、
    /// アイコンファイルの保存が重ならないようにもなる。
    /// </summary>
    private readonly SemaphoreSlim _throttle = new(1, 1);
    private readonly BrowserFaviconCache _browserCache = new();

    /// <summary>
    /// icons/ にあるファイル名の一覧。favicon の有無を調べるたびに
    /// ディスクを見に行かないためのもの。null なら次に必要になったときに読み直す。
    /// </summary>
    private HashSet<string>? _iconFiles;
    private readonly object _iconFilesGate = new();

    private static readonly string[] FaviconExtensions = { ".png", ".ico", ".jpg", ".gif", ".bmp" };

    /// <summary>
    /// フォルダ / ファイルが存在するかの確認結果（キー: "d|パス" または "f|パス"）。
    ///
    /// 一覧を作り直すたびに UI スレッドで存在確認をすると、件数が多いときや
    /// つながらない共有フォルダがあるときに画面が固まる。そこで 2 回目以降は
    /// 前回の結果ですぐに表示し、確認し直しは裏で行う（変わっていればアイコンを差し替える）。
    /// </summary>
    private readonly ConcurrentDictionary<string, ExistenceState> _existence = new(StringComparer.OrdinalIgnoreCase);
    private readonly ConcurrentDictionary<string, byte> _existenceChecks = new(StringComparer.OrdinalIgnoreCase);
    private readonly SemaphoreSlim _existenceThrottle = new(2, 2);

    /// <summary>同じパスの存在を確認し直す間隔。</summary>
    private const long ExistenceRecheckMs = 5000;

    private readonly record struct ExistenceState(bool Exists, long CheckedAt);

    /// <summary>
    /// 共有フォルダ上のフォルダ / ファイルのアイコンを取得する専用スレッドへの依頼。
    ///
    /// フォルダ固有のアイコンや EXE に埋め込まれたアイコンは実物を読まないと分からず、
    /// サーバーの応答が遅いとその間ずっと待たされる。UI スレッドで行うと画面が固まるので、
    /// いったん標準のアイコンを出しておき、取得できたら差し替える。
    /// シェルのアイコン取得は STA のスレッドで行う必要があるため、スレッドプールではなく専用のスレッドを使う。
    /// </summary>
    private readonly BlockingCollection<(Func<ImageSource?> Work, TaskCompletionSource<ImageSource?> Done)> _shellQueue = new();
    private readonly ConcurrentDictionary<string, byte> _shellLoads = new(StringComparer.OrdinalIgnoreCase);
    private readonly object _shellThreadGate = new();
    private Thread? _shellThread;

    public IconService(AppStore store, SettingsService settings)
    {
        _store = store;
        _settings = settings;
    }

    public void Dispose()
    {
        _shellQueue.CompleteAdding();
        _browserCache.Dispose();
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
        item.Icon = ResolveForList(item);

        if (item.TargetType == TargetType.Web && string.IsNullOrEmpty(item.IconPath))
            _ = EnsureFaviconAsync(item, force: false);
    }

    /// <summary>
    /// 一覧に表示するアイコン。
    /// フォルダ / ファイルの存在確認には前回の結果を使い、確認し直しは裏で行う。
    /// </summary>
    private ImageSource ResolveForList(ShortcutItem item)
    {
        if (!string.IsNullOrEmpty(item.IconPath))
        {
            var loaded = LoadFromCache(item.IconPath);
            if (loaded is not null) return loaded;
        }

        var path = TargetResolver.Expand(item.Target ?? string.Empty);
        return item.TargetType switch
        {
            TargetType.Folder => ShellIconForList(path, folder: true) ?? DefaultIcons.Folder,
            TargetType.File => ShellIconForList(path, folder: false) ?? DefaultIcons.File,
            TargetType.Application => ShellIconForList(path, folder: false) ?? DefaultIcons.For(TargetType.Application),
            TargetType.Web => DefaultIcons.Web,
            _ => DefaultIcons.Unknown,
        };
    }

    /// <summary>
    /// フォルダ / ファイルのシェルアイコン。
    /// 共有フォルダ上のもので、実物を読まないと分からないアイコンをまだ取得していなければ、
    /// いったん標準のアイコンを返し、実物のアイコンは裏で取得してから差し替える。
    /// </summary>
    private ImageSource? ShellIconForList(string path, bool folder)
    {
        var exists = KnownExists(path, folder);

        if (exists && NeedsNetworkRead(path, folder, out var placeholder))
        {
            LoadShellIconInBackground(path, folder);
            return placeholder;
        }

        return LoadShellIcon(path, folder, exists);
    }

    private static ImageSource? LoadShellIcon(string path, bool folder, bool exists) =>
        folder ? ShellIconProvider.GetFolderIcon(path, exists) : ShellIconProvider.GetFileIcon(path, exists);

    /// <summary>
    /// 実在するパスのアイコンを出すのに、共有フォルダ上の実物を読む必要があるか。
    /// 必要なら、代わりにすぐ出せる標準のアイコンを <paramref name="placeholder"/> に返す。
    ///
    /// 一般的なファイルのアイコンは拡張子で決まるので、標準のアイコンを取得した時点で
    /// 用が済む（実物を読む必要は無くなる）。フォルダ固有のアイコンや EXE のアイコンは実物を読む。
    /// </summary>
    internal static bool NeedsNetworkRead(string path, bool folder, out ImageSource? placeholder)
    {
        placeholder = null;
        if (ShellIconProvider.IsLoaded(path, folder) || !TargetResolver.IsNetworkPath(path)) return false;

        placeholder = LoadShellIcon(path, folder, exists: false);
        return !ShellIconProvider.IsLoaded(path, folder);
    }

    /// <summary>
    /// パスが存在するか。前回の確認結果があればそれを返し、古くなっていれば裏で確認し直す。
    /// </summary>
    private bool KnownExists(string path, bool folder)
    {
        if (string.IsNullOrWhiteSpace(path)) return false;

        var key = (folder ? "d|" : "f|") + path;
        var now = Environment.TickCount64;

        if (_existence.TryGetValue(key, out var known))
        {
            if (now - known.CheckedAt >= ExistenceRecheckMs) QueueExistenceCheck(key, path, folder);
            return known.Exists;
        }

        // 初めて表示するパス。ローカルはその場で確かめ、最初から正しいアイコンを出す。
        // ネットワーク上のパスはつながらないと長く待たされるので、いったん「無い」ものとして
        // 表示し、裏で確かめてからアイコンを差し替える。
        if (TargetResolver.IsNetworkPath(path))
        {
            _existence[key] = new ExistenceState(false, now);
            QueueExistenceCheck(key, path, folder);
            return false;
        }

        var exists = folder ? Directory.Exists(path) : File.Exists(path);
        _existence[key] = new ExistenceState(exists, Environment.TickCount64);
        return exists;
    }

    private void QueueExistenceCheck(string key, string path, bool folder)
    {
        if (!_existenceChecks.TryAdd(key, 0)) return; // 確認中

        _ = Task.Run(async () =>
        {
            await _existenceThrottle.WaitAsync().ConfigureAwait(false);
            try
            {
                var exists = folder ? Directory.Exists(path) : File.Exists(path);
                var changed = !_existence.TryGetValue(key, out var before) || before.Exists != exists;
                _existence[key] = new ExistenceState(exists, Environment.TickCount64);

                if (changed) RefreshIconsForPath(path);
            }
            catch (Exception ex)
            {
                AppLog.Warn($"存在確認に失敗しました: {path}", ex);
            }
            finally
            {
                _existenceThrottle.Release();
                _existenceChecks.TryRemove(key, out _);
            }
        });
    }

    /// <summary>存在確認の結果が変わったパスについて、表示中のアイコンを差し替える。</summary>
    private void RefreshIconsForPath(string path)
    {
        var app = System.Windows.Application.Current;
        if (app is null) return; // 終了処理中

        app.Dispatcher.InvokeAsync(() =>
        {
            foreach (var item in _store.Shortcuts)
            {
                if (item.Icon is null) continue; // まだ表示していない項目は、表示するときに解決される
                if (item.TargetType is not (TargetType.Folder or TargetType.File or TargetType.Application)) continue;
                if (!string.Equals(TargetResolver.Expand(item.Target ?? string.Empty), path, StringComparison.OrdinalIgnoreCase))
                    continue;

                item.Icon = ResolveForList(item);
            }
        });
    }

    // ------------------------------------------------ 共有フォルダのアイコン

    /// <summary>共有フォルダ上のフォルダ / ファイルのアイコンを裏で取得し、取得できたら一覧の表示を差し替える。</summary>
    private void LoadShellIconInBackground(string path, bool folder)
    {
        var key = (folder ? "d|" : "f|") + path;
        if (!_shellLoads.TryAdd(key, 0)) return; // 取得待ち

        RunOnShellThread(() => LoadShellIcon(path, folder, exists: true))
            .ContinueWith(loaded =>
            {
                _shellLoads.TryRemove(key, out _);
                RefreshIconsForPath(path);
            }, TaskScheduler.Default);
    }

    /// <summary>シェルのアイコン取得を専用スレッドで行う。失敗したときや終了処理中は null。</summary>
    internal Task<ImageSource?> RunOnShellThread(Func<ImageSource?> work)
    {
        var done = new TaskCompletionSource<ImageSource?>(TaskCreationOptions.RunContinuationsAsynchronously);

        lock (_shellThreadGate)
        {
            if (_shellThread is null)
            {
                _shellThread = new Thread(ShellThreadLoop) { IsBackground = true, Name = "ShellIconLoader" };
                _shellThread.SetApartmentState(ApartmentState.STA);
                _shellThread.Start();
            }
        }

        try
        {
            _shellQueue.Add((work, done));
        }
        catch (InvalidOperationException)
        {
            done.TrySetResult(null); // 終了処理中
        }

        return done.Task;
    }

    private void ShellThreadLoop()
    {
        foreach (var (work, done) in _shellQueue.GetConsumingEnumerable())
        {
            try
            {
                done.TrySetResult(work());
            }
            catch (Exception ex)
            {
                AppLog.Warn("共有フォルダのアイコンを取得できませんでした。", ex);
                done.TrySetResult(null);
            }
        }
    }

    /// <summary>
    /// 登録画面のプレビュー用アイコン。存在確認は呼び出し側で済ませた結果を使う。
    ///
    /// 共有フォルダ上のもので実物を読む必要があるアイコンは、いったん標準のアイコンを返し、
    /// 実物のアイコンは裏で取得して <paramref name="pending"/> で渡す（取得できなければ null）。
    /// </summary>
    public ImageSource ResolvePreview(ShortcutItem item, bool exists, out Task<ImageSource?>? pending)
    {
        pending = null;

        if (!string.IsNullOrEmpty(item.IconPath))
        {
            var loaded = LoadFromCache(item.IconPath);
            if (loaded is not null) return loaded;
        }

        var fallback = item.TargetType switch
        {
            TargetType.Folder => DefaultIcons.Folder,
            TargetType.File => DefaultIcons.File,
            TargetType.Application => DefaultIcons.For(TargetType.Application),
            TargetType.Web => DefaultIcons.Web,
            _ => DefaultIcons.Unknown,
        };
        if (item.TargetType is not (TargetType.Folder or TargetType.File or TargetType.Application))
            return fallback;

        var path = TargetResolver.Expand(item.Target ?? string.Empty);
        var folder = item.TargetType == TargetType.Folder;

        if (exists && NeedsNetworkRead(path, folder, out var placeholder))
        {
            pending = RunOnShellThread(() => LoadShellIcon(path, folder, exists: true));
            return placeholder ?? fallback;
        }

        return LoadShellIcon(path, folder, exists) ?? fallback;
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
        _existence.Clear();
        ForgetIconFileList();
        ShellIconProvider.ClearCache();
    }

    // --------------------------------------------------------------- favicon

    /// <summary>
    /// Web ショートカットのアイコンを、ブラウザが保存しているアイコンから取り出して
    /// icons/ に保存し、item に反映する。見つからなければ何もしない（既定アイコンのまま）。
    ///
    /// アプリからサイトへの通信は行わない。ブラウザのファイルは必ず裏のスレッドで読み、
    /// 画面の表示を待たせない。UI スレッドから呼ぶこと。
    /// </summary>
    /// <param name="force">保存済みのアイコンがあっても、ブラウザから取り出し直す。</param>
    /// <returns>アイコンを反映できたら true。</returns>
    public async Task<bool> EnsureFaviconAsync(ShortcutItem item, bool force)
    {
        if (item.TargetType != TargetType.Web) return false;

        // 一覧を表示するたびに呼ばれるので、URL の解析結果は項目ごとに覚えておく
        var web = GetWebTargetInfo(item);

        var pageUri = web.PageUri;
        if (pageUri is null) return false;
        if (pageUri.Scheme != Uri.UriSchemeHttp && pageUri.Scheme != Uri.UriSchemeHttps) return false;

        // http と https、ポート違いは別サイトなのでオリジン単位でキャッシュする
        var origin = web.Origin;
        if (string.IsNullOrEmpty(origin)) return false;

        var fileBase = web.FaviconFileBase!;

        if (!force)
        {
            // すでに保存済みのアイコンがあればそれを使う
            var existing = FindCachedFavicon(fileBase);
            if (existing is not null)
            {
                ApplyIcon(item, existing);
                return true;
            }
            if (_failed.ContainsKey(origin)) return false;
        }

        if (!_settings.Current.UseBrowserIconCache) return false;

        // 同じサイトを探している最中なら任せる。ただし利用者が明示的に取り直したときは、
        // その結果を伝える必要があるので自分でも探す（読み取りは 1 件ずつなので重ならない）
        var tracked = _inFlight.TryAdd(origin, 0);
        if (!tracked && !force) return false;

        try
        {
            var saved = await FindInBrowserCacheAsync(pageUri, fileBase).ConfigureAwait(false);
            if (saved is null)
            {
                _failed[origin] = 0;
                return false;
            }

            var app = System.Windows.Application.Current;
            if (app is null) return false; // 終了処理中

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
                    if (!string.Equals(GetWebTargetInfo(other).Origin, origin, StringComparison.OrdinalIgnoreCase))
                        continue;
                    ApplyIcon(other, saved);
                }
            });
            return true;
        }
        catch (Exception ex)
        {
            AppLog.Warn($"サイトのアイコンを取得できませんでした: {origin}", ex);
            _failed[origin] = 0;
            return false;
        }
        finally
        {
            if (tracked) _inFlight.TryRemove(origin, out _);
        }
    }

    /// <summary>ブラウザのアイコンキャッシュを裏のスレッドで探し、見つかれば icons/ に保存したファイル名を返す。</summary>
    private async Task<string?> FindInBrowserCacheAsync(Uri pageUri, string fileBase)
    {
        await _throttle.WaitAsync().ConfigureAwait(false);
        try
        {
            // ブラウザのファイルのコピーを伴うので、呼び出し元が UI スレッドでも必ず裏で行う
            return await Task.Run(() => TryBrowserCache(pageUri, fileBase)).ConfigureAwait(false);
        }
        finally
        {
            _throttle.Release();
        }
    }

    /// <summary>ブラウザのアイコンキャッシュから取り出して icons/ に保存する。</summary>
    private string? TryBrowserCache(Uri pageUri, string fileBase)
    {
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
            var dark = SystemTheme.IsDark(_settings.Current.Theme);

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

                var app = System.Windows.Application.Current;
                if (app is null) return; // 終了処理中

                // 項目を見たり書き換えたりするのは UI スレッドで行う（ブラウザのファイルを読むのは裏）
                var found = await app.Dispatcher.InvokeAsync(() =>
                {
                    if (!string.IsNullOrEmpty(item.IconPath)) return Task.FromResult(true);

                    _failed.TryRemove(origin, out _);
                    return EnsureFaviconAsync(item, force: false);
                }).Task.Unwrap().ConfigureAwait(false);

                if (found) return;
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
    private string? SaveIconBytes(byte[] bytes, string fileBase)
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
        {
            if (old.EndsWith(".tmp", StringComparison.Ordinal)) continue;
            File.Delete(old);
            ForgetIconFile(Path.GetFileName(old));
        }

        File.Move(temp, full, overwrite: true);
        RememberIconFile(fileName);
        return fileName;
    }

    private string? FindCachedFavicon(string fileBase)
    {
        foreach (var ext in FaviconExtensions)
        {
            var name = fileBase + ext;
            if (IconFileExists(name)) return name;
        }
        return null;
    }

    /// <summary>URL の解析結果。項目の URL が変わっていなければ前回の結果を使う。</summary>
    private static WebTargetInfo GetWebTargetInfo(ShortcutItem item)
    {
        var target = item.Target;
        var info = item.WebTargetInfo;
        if (info is not null && ReferenceEquals(info.Target, target)) return info;

        var origin = TargetResolver.TryGetOrigin(target);
        info = new WebTargetInfo
        {
            Target = target,
            PageUri = TargetResolver.TryGetUri(target),
            Origin = origin,
            FaviconFileBase = string.IsNullOrEmpty(origin) ? null : "fav_" + Hash(origin),
        };
        item.WebTargetInfo = info;
        return info;
    }

    // ------------------------------------------------------- icons/ の一覧

    /// <summary>
    /// icons/ にそのファイルがあるか。一覧はメモリに持っておき、
    /// 「ある」と判断したときだけ実物を確かめる（無いものを毎回ディスクに問い合わせない）。
    /// </summary>
    private bool IconFileExists(string name)
    {
        lock (_iconFilesGate)
        {
            _iconFiles ??= LoadIconFileNames();
            if (!_iconFiles.Contains(name)) return false;
        }

        if (File.Exists(FullPath(name))) return true;

        ForgetIconFile(name);
        return false;
    }

    private static HashSet<string> LoadIconFileNames()
    {
        var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        try
        {
            if (Directory.Exists(AppPaths.IconDirectory))
                foreach (var file in Directory.EnumerateFiles(AppPaths.IconDirectory))
                    names.Add(Path.GetFileName(file));
        }
        catch (Exception ex)
        {
            AppLog.Warn("アイコンフォルダの一覧を読めませんでした。", ex);
        }
        return names;
    }

    private void RememberIconFile(string name)
    {
        lock (_iconFilesGate) _iconFiles?.Add(name);
    }

    private void ForgetIconFile(string name)
    {
        lock (_iconFilesGate) _iconFiles?.Remove(name);
    }

    /// <summary>一覧を捨てて、次に必要になったときに読み直させる。</summary>
    private void ForgetIconFileList()
    {
        lock (_iconFilesGate) _iconFiles = null;
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

            RememberIconFile(fileName);
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
            ForgetIconFile(relativeFileName);
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
        ForgetIconFileList();
        return removed;
    }
}

using System.IO;
using System.Windows;
using System.Windows.Input;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;
using FavoriteShortcut.Views;
using Forms = System.Windows.Forms;

namespace FavoriteShortcut;

public partial class App : Application
{
    private const string MutexName = @"Local\FavoriteShortcut.SingleInstance";

    private Mutex? _mutex;
    private Forms.NotifyIcon? _tray;
    private MainWindow? _mainWindow;
    private LauncherWindow? _launcherWindow;
    private bool _shuttingDown;

    /// <summary>アプリ全体で共有するサービスへの入口。</summary>
    public static App Instance => (App)Current;

    public Database Db { get; private set; } = null!;
    public AppStore Store { get; private set; } = null!;
    public SettingsService Settings { get; private set; } = null!;
    public IconService Icons { get; private set; } = null!;
    public ExportImportService Transfer { get; private set; } = null!;
    public HotKeyService HotKeys { get; private set; } = null!;

    /// <summary>ホットキーの登録に失敗したまま起動した場合 true（設定画面で注意表示）。</summary>
    public bool HotKeyRegistrationFailed { get; private set; }

    /// <summary>
    /// 終了処理に入ったかどうか。各ウィンドウは、これが true のときは
    /// 「閉じる＝トレイへ格納」ではなく本当に閉じる。
    /// </summary>
    public bool IsExiting => _shuttingDown;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        // 二重起動を防ぐ。すでに動いていれば、そちらのウィンドウを前に出して終了する。
        _mutex = new Mutex(initiallyOwned: true, MutexName, out var isFirstInstance);
        if (!isFirstInstance)
        {
            HotKeyService.BroadcastShowRequest();
            Shutdown();
            return;
        }

        DispatcherUnhandledException += OnUnhandledException;
        AppDomain.CurrentDomain.UnhandledException += (_, args) =>
            AppLog.Error("処理されない例外", args.ExceptionObject as Exception);

        try
        {
            InitializeServices();
        }
        catch (Exception ex)
        {
            AppLog.Error("起動処理に失敗しました。", ex);
            MessageBox.Show(
                "データベースを開けませんでした。\n\n" +
                $"保存場所: {AppPaths.DataDirectory}\n\n{ex.Message}",
                "お気に入りショートカット", MessageBoxButton.OK, MessageBoxImage.Error);
            Shutdown();
            return;
        }

        var startHidden = e.Args.Any(a =>
            string.Equals(a, StartupService.TrayArgument, StringComparison.OrdinalIgnoreCase))
            || Settings.Current.StartMinimized;

        if (!startHidden) ShowMainWindow();

        AppLog.Info($"起動しました（データ保存場所: {AppPaths.DataDirectory}）");
    }

    private void InitializeServices()
    {
        Db = new Database(AppPaths.DatabaseFile);
        Store = new AppStore(Db);
        Settings = new SettingsService(Store);
        Icons = new IconService(Store, Settings);
        Transfer = new ExportImportService(Store, Settings);

        HookWindowTheme();
        ApplyTheme(Settings.Current.Theme);

        if (Store.AllFolders.Count == 0 && Store.Shortcuts.Count == 0)
            SeedInitialData();

        HotKeys = new HotKeyService();
        HotKeys.HotKeyPressed += (_, _) => ToggleLauncher();
        HotKeys.ShowRequested += (_, _) => ShowMainWindow();
        RegisterHotKeyFromSettings();

        CreateTrayIcon();
    }

    /// <summary>初回起動時に、使い方が分かる最小限のフォルダを用意する。</summary>
    private void SeedInitialData()
    {
        try
        {
            foreach (var name in new[] { "仕事", "開発", "個人", "その他" })
                Store.CreateFolder(name, null);
        }
        catch (Exception ex)
        {
            AppLog.Warn("初期フォルダの作成に失敗しました。", ex);
        }
    }

    public bool RegisterHotKeyFromSettings()
    {
        var modifiers = (ModifierKeys)Settings.Current.HotKeyModifiers;
        var key = (Key)Settings.Current.HotKeyKey;
        var ok = HotKeys.Register(modifiers, key);
        HotKeyRegistrationFailed = !ok;
        return ok;
    }

    // ------------------------------------------------------------- ウィンドウ

    public void ShowMainWindow()
    {
        if (_mainWindow is null)
        {
            _mainWindow = new MainWindow();
            _mainWindow.Closed += (_, _) => _mainWindow = null;
        }

        if (!_mainWindow.IsVisible) _mainWindow.Show();
        if (_mainWindow.WindowState == WindowState.Minimized)
            _mainWindow.WindowState = WindowState.Normal;

        _mainWindow.Activate();
        _mainWindow.Focus();
    }

    /// <summary>ランチャーの表示 / 非表示を切り替える（§53）。</summary>
    public void ToggleLauncher()
    {
        _launcherWindow ??= new LauncherWindow();

        if (_launcherWindow.IsVisible)
        {
            _launcherWindow.HideLauncher();
            return;
        }

        _launcherWindow.ShowLauncher();
    }

    public void ShowLauncher()
    {
        _launcherWindow ??= new LauncherWindow();
        _launcherWindow.ShowLauncher();
    }

    public void ApplyTheme(AppTheme theme)
    {
        var source = new Uri(theme == AppTheme.Dark ? "Themes/Dark.xaml" : "Themes/Light.xaml",
            UriKind.Relative);
        Resources.MergedDictionaries[0] = new ResourceDictionary { Source = source };
        WindowThemeHelper.ApplyToAll(theme == AppTheme.Dark);
    }

    /// <summary>
    /// どのウィンドウが開かれても、タイトルバーの配色を現在のテーマに合わせる。
    /// 個々のウィンドウに処理を書かずに済むよう、クラスハンドラで一括登録している。
    /// </summary>
    private void HookWindowTheme()
    {
        EventManager.RegisterClassHandler(typeof(Window), FrameworkElement.LoadedEvent,
            new RoutedEventHandler((sender, _) =>
            {
                if (sender is Window window)
                    WindowThemeHelper.Apply(window, Settings.Current.Theme == AppTheme.Dark);
            }));
    }

    // ------------------------------------------------------------ タスクトレイ

    private void CreateTrayIcon()
    {
        try
        {
            var menu = new Forms.ContextMenuStrip();
            menu.Items.Add("管理画面を開く(&O)", null, (_, _) => Dispatcher.Invoke(ShowMainWindow));
            menu.Items.Add("ランチャーを表示(&L)", null, (_, _) => Dispatcher.Invoke(ShowLauncher));
            menu.Items.Add(new Forms.ToolStripSeparator());
            menu.Items.Add("設定(&S)", null, (_, _) => Dispatcher.Invoke(OpenSettings));
            menu.Items.Add(new Forms.ToolStripSeparator());
            menu.Items.Add("終了(&X)", null, (_, _) => Dispatcher.Invoke(ExitApplication));

            _tray = new Forms.NotifyIcon
            {
                Icon = LoadTrayIcon(),
                Text = "お気に入りショートカット",
                Visible = true,
                ContextMenuStrip = menu,
            };
            _tray.DoubleClick += (_, _) => Dispatcher.Invoke(ShowMainWindow);
        }
        catch (Exception ex)
        {
            AppLog.Warn("タスクトレイアイコンを作成できませんでした。", ex);
        }
    }

    private static System.Drawing.Icon LoadTrayIcon()
    {
        var stream = GetResourceStream(new Uri("Assets/app.ico", UriKind.Relative))?.Stream;
        if (stream is not null)
        {
            using (stream)
            {
                using var memory = new MemoryStream();
                stream.CopyTo(memory);
                memory.Position = 0;
                return new System.Drawing.Icon(memory, new System.Drawing.Size(16, 16));
            }
        }

        return System.Drawing.SystemIcons.Application;
    }

    private void OpenSettings()
    {
        ShowMainWindow();
        _mainWindow?.OpenSettings();
    }

    public void ShowTrayMessage(string title, string text)
    {
        try
        {
            _tray?.ShowBalloonTip(3000, title, text, Forms.ToolTipIcon.Info);
        }
        catch (Exception ex)
        {
            AppLog.Warn("通知の表示に失敗しました。", ex);
        }
    }

    /// <summary>ウィンドウを閉じたときに、終了せずトレイへ格納するか。</summary>
    public bool ShouldStayInTray => !_shuttingDown && Settings.Current.MinimizeToTray && _tray is not null;

    public void ExitApplication()
    {
        if (_shuttingDown) return;
        _shuttingDown = true;

        try
        {
            _mainWindow?.PersistWindowState();
            _launcherWindow?.Close();
            _mainWindow?.Close();

            if (_tray is not null)
            {
                _tray.Visible = false;
                _tray.Dispose();
                _tray = null;
            }

            HotKeys?.Dispose();
            Store?.Dispose();
        }
        catch (Exception ex)
        {
            AppLog.Error("終了処理でエラーが発生しました。", ex);
        }

        AppLog.Info("終了しました。");
        Shutdown();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        try
        {
            _mutex?.ReleaseMutex();
            _mutex?.Dispose();
        }
        catch
        {
            // 取得できていない場合の ReleaseMutex 例外は無視してよい
        }

        base.OnExit(e);
    }

    private void OnUnhandledException(object sender,
        System.Windows.Threading.DispatcherUnhandledExceptionEventArgs e)
    {
        AppLog.Error("UIスレッドで処理されない例外が発生しました。", e.Exception);

        MessageBox.Show(
            "予期しないエラーが発生しました。アプリは動作を続けます。\n\n" +
            $"{e.Exception.Message}\n\n詳細はログをご確認ください:\n{AppPaths.LogFile}",
            "お気に入りショートカット", MessageBoxButton.OK, MessageBoxImage.Warning);

        // データは都度 SQLite に保存済みなので、落とさずに続行する
        e.Handled = true;
    }
}

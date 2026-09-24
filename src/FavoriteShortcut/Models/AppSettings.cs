namespace FavoriteShortcut.Models;

public enum ShortcutViewMode
{
    Card = 0,
    List = 1,
}

public enum AppTheme
{
    Light = 0,
    Dark = 1,
}

/// <summary>アプリ設定。DB の settings テーブル（key/value）に保存される。</summary>
public sealed class AppSettings
{
    public ShortcutViewMode ViewMode { get; set; } = ShortcutViewMode.Card;

    /// <summary>カード/リストのアイコン表示サイズ（px）。</summary>
    public int IconSize { get; set; } = 32;

    public AppTheme Theme { get; set; } = AppTheme.Light;

    /// <summary>ランチャー用グローバルホットキー（System.Windows.Input.ModifierKeys の値）。</summary>
    public int HotKeyModifiers { get; set; } = 2; // Control

    /// <summary>ランチャー用グローバルホットキー（System.Windows.Input.Key の値）。</summary>
    public int HotKeyKey { get; set; } = 18; // Key.Space

    /// <summary>ウィンドウを閉じたときにタスクトレイへ常駐するか。</summary>
    public bool MinimizeToTray { get; set; } = true;

    /// <summary>起動時にメインウィンドウを表示せずトレイ常駐で始めるか。</summary>
    public bool StartMinimized { get; set; }

    /// <summary>Windows ログオン時に自動起動するか。</summary>
    public bool RunAtStartup { get; set; }

    /// <summary>
    /// ブラウザ（Chrome / Edge / Firefox など）が保存しているアイコンキャッシュを参照するか。
    /// 通信を伴わず、認証が必要な社内サイトのアイコンも表示できる。
    /// </summary>
    public bool UseBrowserIconCache { get; set; } = true;

    /// <summary>ランチャーで検索語が空のときに最近使った項目を出すか。</summary>
    public bool ShowRecentInLauncher { get; set; } = true;

    public int LauncherMaxResults { get; set; } = 12;

    /// <summary>起動時に最後に選んでいたフォルダを復元するか。</summary>
    public bool RestoreLastFolder { get; set; } = true;

    public string? LastFolderId { get; set; }

    public double MainWindowWidth { get; set; } = 1040;
    public double MainWindowHeight { get; set; } = 680;
    public double FolderPaneWidth { get; set; } = 240;

    public AppSettings Clone() => (AppSettings)MemberwiseClone();
}

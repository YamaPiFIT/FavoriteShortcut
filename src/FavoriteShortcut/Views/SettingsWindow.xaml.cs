using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;
using Microsoft.Win32;

namespace FavoriteShortcut.Views;

/// <summary>設定画面（§39, §63, §66）。</summary>
public partial class SettingsWindow : Window
{
    private static readonly int[] MaxResultChoices = { 6, 8, 12, 20, 30 };

    private readonly App _app = App.Instance;
    private readonly SettingsService _settings;

    /// <summary>キー入力を待っている呼び出しキー。</summary>
    private enum HotKeyTarget
    {
        None,
        Launcher,
        Clipboard,
    }

    private ModifierKeys _hotKeyModifiers;
    private Key _hotKeyKey;
    private ModifierKeys _clipboardHotKeyModifiers;
    private Key _clipboardHotKeyKey;
    private HotKeyTarget _capturing;
    private string? _pendingDataDirectory;
    private bool _resetDataDirectory;
    private bool _loaded;

    public SettingsWindow()
    {
        InitializeComponent();
        _settings = _app.Settings;

        BuildMaxResultChoices();
        LoadSettings();
        _loaded = true;

        Loc.LanguageChanged += OnAppLanguageChanged;
    }

    private void LoadSettings()
    {
        var s = _settings.Current;

        SelectByTag(LanguageCombo, (int)s.Language, fallbackIndex: 0);
        SelectByTag(ThemeCombo, (int)s.Theme, fallbackIndex: 0);
        ViewModeCombo.SelectedIndex = s.ViewMode == ShortcutViewMode.List ? 1 : 0;
        SelectByTag(IconSizeCombo, s.IconSize, fallbackIndex: 1);
        SelectByTag(MaxResultsCombo, s.LauncherMaxResults, fallbackIndex: 2);

        _hotKeyModifiers = (ModifierKeys)s.HotKeyModifiers;
        _hotKeyKey = (Key)s.HotKeyKey;
        _clipboardHotKeyModifiers = (ModifierKeys)s.ClipboardHotKeyModifiers;
        _clipboardHotKeyKey = (Key)s.ClipboardHotKeyKey;

        ShowRecentCheck.IsChecked = s.ShowRecentInLauncher;
        MinimizeToTrayCheck.IsChecked = s.MinimizeToTray;
        StartMinimizedCheck.IsChecked = s.StartMinimized;
        RestoreFolderCheck.IsChecked = s.RestoreLastFolder;
        BrowserIconCacheCheck.IsChecked = s.UseBrowserIconCache;
        AutoBackupCheck.IsChecked = s.AutoBackup;
        RunAtStartupCheck.IsChecked = StartupService.IsEnabled();

        RefreshLocalizedTexts();
    }

    /// <summary>
    /// コードで組み立てている文言を、現在の表示言語で作り直す。
    /// （XAML に書いた文言は DynamicResource なので自動で切り替わる）
    /// </summary>
    private void RefreshLocalizedTexts()
    {
        LanguageNote.Text = Loc.DescribeAutoDetection();
        ThemeNote.Text = Loc.T("Str.Settings.ThemeAutoNote", Loc.T(SystemTheme.WindowsUsesDarkMode()
            ? "Str.Settings.ThemeDark"
            : "Str.Settings.ThemeLight"));

        foreach (var obj in MaxResultsCombo.Items)
        {
            if (obj is ComboBoxItem { Tag: string tag } item && int.TryParse(tag, out var count))
                item.Content = Loc.T("Str.Common.CountItems", count);
        }

        if (_capturing == HotKeyTarget.None)
        {
            CaptureButton.Content = Loc.T("Str.Settings.Change");
            ClipboardCaptureButton.Content = Loc.T("Str.Settings.Change");
            UpdateHotKeyDisplay();
        }

        BrowserIconCacheDetected.Text = DescribeDetectedBrowsers();
        CleanIconsResult.Text = string.Empty;

        StartupStatus.Text = Loc.T("Str.Settings.RunAtStartupNote");

        UpdateDataPathText();
        DataPathNote.Text = Loc.T(AppPaths.UsingFallbackLocation
            ? "Str.Settings.DataNoteFallback"
            : "Str.Settings.DataNoteDefault");

        var lastBackup = AutoBackupService.LastBackupTime;
        AutoBackupNote.Text = Loc.T("Str.Settings.AutoBackupNote", AutoBackupService.KeepCount) + "\n" +
                              (lastBackup is { } time
                                  ? Loc.T("Str.Settings.AutoBackupLast", time.ToString("g", CultureInfo.CurrentCulture))
                                  : Loc.T("Str.Settings.AutoBackupNone"));

        var version = typeof(SettingsWindow).Assembly.GetName().Version?.ToString(3) ?? "1.0.0";
        VersionText.Text = Loc.T("Str.Settings.Version", Loc.T("Str.App.Name"), version);
        StatsText.Text = Loc.T("Str.Settings.Stats",
            _app.Store.Shortcuts.Count, _app.Store.AllFolders.Count, _app.Store.AllTagNames.Count());
    }

    private void BuildMaxResultChoices()
    {
        MaxResultsCombo.Items.Clear();
        foreach (var count in MaxResultChoices)
        {
            MaxResultsCombo.Items.Add(new ComboBoxItem
            {
                Tag = count.ToString(CultureInfo.InvariantCulture),
                Content = Loc.T("Str.Common.CountItems", count),
            });
        }
    }

    private static void SelectByTag(ComboBox combo, int value, int fallbackIndex)
    {
        foreach (var obj in combo.Items)
        {
            if (obj is not ComboBoxItem item) continue;
            if (item.Tag is string tag &&
                int.TryParse(tag, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed) &&
                parsed == value)
            {
                combo.SelectedItem = item;
                return;
            }
        }
        combo.SelectedIndex = fallbackIndex;
    }

    private static int TagValue(ComboBox combo, int fallback) =>
        combo.SelectedItem is ComboBoxItem { Tag: string tag } &&
        int.TryParse(tag, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed)
            ? parsed
            : fallback;

    private AppTheme SelectedTheme
    {
        get
        {
            var value = TagValue(ThemeCombo, (int)AppTheme.Auto);
            return Enum.IsDefined(typeof(AppTheme), value) ? (AppTheme)value : AppTheme.Auto;
        }
    }

    private AppLanguage SelectedLanguage
    {
        get
        {
            var value = TagValue(LanguageCombo, (int)AppLanguage.Auto);
            return Enum.IsDefined(typeof(AppLanguage), value) ? (AppLanguage)value : AppLanguage.Auto;
        }
    }

    // ---------------------------------------------------------- 表示・言語

    /// <summary>テーマは選んだ瞬間に反映して、見た目を確認できるようにする。</summary>
    private void OnThemeChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!_loaded) return;
        _app.ApplyTheme(SelectedTheme);
    }

    /// <summary>言語も選んだ瞬間に切り替える（キャンセルすれば元に戻る）。</summary>
    private void OnLanguageChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!_loaded) return;
        Loc.Apply(SelectedLanguage);
    }

    private void OnAppLanguageChanged(object? sender, EventArgs e) => RefreshLocalizedTexts();

    // ------------------------------------------------------------- ホットキー

    private void OnCaptureHotKey(object sender, RoutedEventArgs e) => ToggleCapturing(HotKeyTarget.Launcher);

    private void OnCaptureClipboardHotKey(object sender, RoutedEventArgs e) => ToggleCapturing(HotKeyTarget.Clipboard);

    /// <summary>「クリップボードから登録」のキーを未設定に戻す。</summary>
    private void OnClearClipboardHotKey(object sender, RoutedEventArgs e)
    {
        _clipboardHotKeyModifiers = ModifierKeys.None;
        _clipboardHotKeyKey = Key.None;
        StopCapturing();
    }

    private void ToggleCapturing(HotKeyTarget target)
    {
        if (_capturing == target)
        {
            StopCapturing();
            return;
        }

        StopCapturing();
        _capturing = target;

        var (button, text, status) = target == HotKeyTarget.Launcher
            ? (CaptureButton, HotKeyText, HotKeyStatus)
            : (ClipboardCaptureButton, ClipboardHotKeyText, ClipboardHotKeyStatus);
        button.Content = Loc.T("Str.Common.Cancel");
        text.Text = Loc.T("Str.Settings.PressKeys");
        status.Text = Loc.T("Str.Settings.CaptureHint");
        Keyboard.Focus(this);
    }

    private void StopCapturing()
    {
        _capturing = HotKeyTarget.None;
        CaptureButton.Content = Loc.T("Str.Settings.Change");
        ClipboardCaptureButton.Content = Loc.T("Str.Settings.Change");
        UpdateHotKeyDisplay();
    }

    protected override void OnPreviewKeyDown(KeyEventArgs e)
    {
        if (_capturing == HotKeyTarget.None)
        {
            base.OnPreviewKeyDown(e);
            return;
        }

        e.Handled = true;

        var key = e.Key == Key.System ? e.SystemKey : e.Key;

        if (key == Key.Escape)
        {
            StopCapturing();
            return;
        }

        // 修飾キー単体は確定しない
        if (key is Key.LeftCtrl or Key.RightCtrl or Key.LeftShift or Key.RightShift
            or Key.LeftAlt or Key.RightAlt or Key.LWin or Key.RWin
            or Key.ImeProcessed or Key.DeadCharProcessed or Key.None)
            return;

        var modifiers = Keyboard.Modifiers;
        var status = _capturing == HotKeyTarget.Launcher ? HotKeyStatus : ClipboardHotKeyStatus;
        if (modifiers == ModifierKeys.None)
        {
            status.Text = Loc.T("Str.Settings.NeedModifier");
            return;
        }

        // 2 つの呼び出しキーに同じ組み合わせは使えない
        if (_capturing == HotKeyTarget.Launcher)
        {
            if (modifiers == _clipboardHotKeyModifiers && key == _clipboardHotKeyKey)
            {
                status.Text = Loc.T("Str.Settings.HotKeySameAsClipboard");
                return;
            }

            _hotKeyModifiers = modifiers;
            _hotKeyKey = key;
        }
        else
        {
            if (modifiers == _hotKeyModifiers && key == _hotKeyKey)
            {
                status.Text = Loc.T("Str.Settings.HotKeySameAsLauncher");
                return;
            }

            _clipboardHotKeyModifiers = modifiers;
            _clipboardHotKeyKey = key;
        }

        StopCapturing();
    }

    private void UpdateHotKeyDisplay()
    {
        // 登録に失敗したキーをそのまま表示しているときだけ、使用中である旨を出す
        HotKeyText.Text = HotKeyService.Describe(_hotKeyModifiers, _hotKeyKey);
        var launcherUnchanged = (ModifierKeys)_settings.Current.HotKeyModifiers == _hotKeyModifiers &&
                                (Key)_settings.Current.HotKeyKey == _hotKeyKey;
        HotKeyStatus.Text = Loc.T(_app.HotKeyRegistrationFailed && launcherUnchanged
            ? "Str.Settings.HotKeyInUse"
            : "Str.Settings.HotKeyConflictHint");

        ClipboardHotKeyText.Text = HotKeyService.Describe(_clipboardHotKeyModifiers, _clipboardHotKeyKey);
        var clipboardUnchanged = (ModifierKeys)_settings.Current.ClipboardHotKeyModifiers == _clipboardHotKeyModifiers &&
                                 (Key)_settings.Current.ClipboardHotKeyKey == _clipboardHotKeyKey;
        ClipboardHotKeyStatus.Text = Loc.T(_app.ClipboardHotKeyRegistrationFailed && clipboardUnchanged
            ? "Str.Settings.HotKeyInUse"
            : "Str.Settings.ClipboardHotKeyNote");
    }

    // ------------------------------------------------------------- アイコン

    /// <summary>この PC で参照できるブラウザを表示する（設定の効果を分かりやすくするため）。</summary>
    private static string DescribeDetectedBrowsers()
    {
        var browsers = BrowserFaviconCache.DetectAvailableBrowsers();
        return browsers.Count == 0
            ? Loc.T("Str.Settings.NoBrowsers")
            : Loc.T("Str.Settings.BrowsersFound", string.Join(Loc.T("Str.Common.ListSeparator"), browsers));
    }

    private void OnCleanIcons(object sender, RoutedEventArgs e)
    {
        var removed = _app.Icons.CleanUpUnusedIcons();
        CleanIconsResult.Text = removed == 0
            ? Loc.T("Str.Settings.CleanNone")
            : Loc.T("Str.Settings.CleanDone", removed);
    }

    // ------------------------------------------------------------- データ

    private void OnOpenDataFolder(object sender, RoutedEventArgs e) =>
        LaunchService.OpenPath(AppPaths.DataDirectory);

    private void OnOpenBackupFolder(object sender, RoutedEventArgs e) =>
        LaunchService.OpenPath(AppPaths.BackupDirectory);

    private void OnOpenLog(object sender, RoutedEventArgs e)
    {
        if (System.IO.File.Exists(AppPaths.LogFile)) LaunchService.OpenPath(AppPaths.LogFile);
        else MessageBox.Show(this, Loc.T("Str.Settings.NoLog"), Loc.T("Str.Settings.LogTitle"),
            MessageBoxButton.OK, MessageBoxImage.Information);
    }

    private void OnChangeDataFolder(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog
        {
            Title = Loc.T("Str.Settings.ChooseDataFolder"),
            InitialDirectory = AppPaths.DataDirectory,
        };
        if (dialog.ShowDialog(this) != true) return;

        _pendingDataDirectory = dialog.FolderName;
        _resetDataDirectory = false;
        UpdateDataPathText();
    }

    private void OnResetDataFolder(object sender, RoutedEventArgs e)
    {
        _pendingDataDirectory = null;
        _resetDataDirectory = true;
        UpdateDataPathText();
    }

    /// <summary>保存場所の表示。変更を予約していれば「次回起動から」を添える。</summary>
    private void UpdateDataPathText()
    {
        DataPathText.Text = _resetDataDirectory
            ? Loc.T("Str.Settings.NextLaunchDefault", AppPaths.DataDirectory, AppPaths.DefaultDataDirectory)
            : _pendingDataDirectory is not null
                ? Loc.T("Str.Settings.NextLaunch", AppPaths.DataDirectory, _pendingDataDirectory)
                : AppPaths.DataDirectory;
    }

    // ---------------------------------------------------------------- 保存

    private void OnSave(object sender, RoutedEventArgs e)
    {
        var s = _settings.Current.Clone();

        s.Language = SelectedLanguage;
        s.Theme = SelectedTheme;
        s.ViewMode = ViewModeCombo.SelectedIndex == 1 ? ShortcutViewMode.List : ShortcutViewMode.Card;
        s.IconSize = TagValue(IconSizeCombo, 32);
        s.LauncherMaxResults = TagValue(MaxResultsCombo, 12);
        s.HotKeyModifiers = (int)_hotKeyModifiers;
        s.HotKeyKey = (int)_hotKeyKey;
        s.ClipboardHotKeyModifiers = (int)_clipboardHotKeyModifiers;
        s.ClipboardHotKeyKey = (int)_clipboardHotKeyKey;
        s.ShowRecentInLauncher = ShowRecentCheck.IsChecked == true;
        s.MinimizeToTray = MinimizeToTrayCheck.IsChecked == true;
        s.StartMinimized = StartMinimizedCheck.IsChecked == true;
        s.RestoreLastFolder = RestoreFolderCheck.IsChecked == true;
        s.UseBrowserIconCache = BrowserIconCacheCheck.IsChecked == true;
        s.AutoBackup = AutoBackupCheck.IsChecked == true;
        s.RunAtStartup = RunAtStartupCheck.IsChecked == true;

        _settings.Save(s);
        _app.ApplyTheme(s.Theme);
        Loc.Apply(s.Language);

        // 2 つのキーを入れ替えた場合に備え、先に「クリップボードから登録」を解除してから登録し直す
        _app.HotKeys.RegisterClipboard(ModifierKeys.None, Key.None);

        if (!_app.RegisterHotKeyFromSettings())
        {
            MessageBox.Show(this,
                Loc.T("Str.Settings.HotKeyFailed", HotKeyService.Describe(_hotKeyModifiers, _hotKeyKey)),
                Loc.T("Str.Settings.HotKeyTitle"), MessageBoxButton.OK, MessageBoxImage.Warning);
        }

        if (!_app.RegisterClipboardHotKeyFromSettings())
        {
            MessageBox.Show(this,
                Loc.T("Str.Settings.ClipboardHotKeyFailed",
                    HotKeyService.Describe(_clipboardHotKeyModifiers, _clipboardHotKeyKey)),
                Loc.T("Str.Settings.HotKeyTitle"), MessageBoxButton.OK, MessageBoxImage.Warning);
        }

        if (StartupService.IsEnabled() != s.RunAtStartup && !StartupService.SetEnabled(s.RunAtStartup))
        {
            MessageBox.Show(this, Loc.T("Str.Settings.StartupFailed"),
                Loc.T("Str.Settings.StartupTitle"), MessageBoxButton.OK, MessageBoxImage.Warning);
        }

        if (_resetDataDirectory || _pendingDataDirectory is not null)
        {
            try
            {
                AppPaths.SaveDataDirectoryOverride(_resetDataDirectory ? null : _pendingDataDirectory);
                MessageBox.Show(this, Loc.T("Str.Settings.LocationChanged"),
                    Loc.T("Str.Settings.LocationTitle"), MessageBoxButton.OK, MessageBoxImage.Information);
            }
            catch (Exception ex)
            {
                AppLog.Error("保存場所の変更に失敗しました。", ex);
                MessageBox.Show(this, Loc.T("Str.Settings.LocationFailed", ex.Message),
                    Loc.T("Str.Common.Error"), MessageBoxButton.OK, MessageBoxImage.Error);
            }
        }

        DialogResult = true;
    }

    protected override void OnClosed(EventArgs e)
    {
        Loc.LanguageChanged -= OnAppLanguageChanged;

        // キャンセルで閉じたときは、プレビュー用に変えていたテーマと言語を元に戻す
        if (DialogResult != true)
        {
            _app.ApplyTheme(_settings.Current.Theme);
            Loc.Apply(_settings.Current.Language);
        }

        base.OnClosed(e);
    }
}

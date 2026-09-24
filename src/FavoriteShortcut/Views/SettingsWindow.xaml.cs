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
    private readonly App _app = App.Instance;
    private readonly SettingsService _settings;

    private ModifierKeys _hotKeyModifiers;
    private Key _hotKeyKey;
    private bool _capturing;
    private string? _pendingDataDirectory;
    private bool _resetDataDirectory;
    private bool _loaded;

    public SettingsWindow()
    {
        InitializeComponent();
        _settings = _app.Settings;
        LoadSettings();
        _loaded = true;
    }

    private void LoadSettings()
    {
        var s = _settings.Current;

        ThemeCombo.SelectedIndex = s.Theme == AppTheme.Dark ? 1 : 0;
        ViewModeCombo.SelectedIndex = s.ViewMode == ShortcutViewMode.List ? 1 : 0;
        SelectByTag(IconSizeCombo, s.IconSize, fallbackIndex: 1);
        SelectByTag(MaxResultsCombo, s.LauncherMaxResults, fallbackIndex: 2);

        _hotKeyModifiers = (ModifierKeys)s.HotKeyModifiers;
        _hotKeyKey = (Key)s.HotKeyKey;
        UpdateHotKeyDisplay();

        ShowRecentCheck.IsChecked = s.ShowRecentInLauncher;
        MinimizeToTrayCheck.IsChecked = s.MinimizeToTray;
        StartMinimizedCheck.IsChecked = s.StartMinimized;
        RestoreFolderCheck.IsChecked = s.RestoreLastFolder;
        BrowserIconCacheCheck.IsChecked = s.UseBrowserIconCache;
        BrowserIconCacheDetected.Text = DescribeDetectedBrowsers();

        RunAtStartupCheck.IsChecked = StartupService.IsEnabled();
        StartupStatus.Text = "EXE を別の場所へ移動した場合は、自動起動をいったんオフにしてから入れ直してください。";

        DataPathText.Text = AppPaths.DataDirectory;
        DataPathNote.Text = AppPaths.UsingFallbackLocation
            ? "EXE のある場所には書き込めないため、上記の場所を使用しています。" +
              "EXE と同じ場所の Data フォルダにまとめたい場合は、EXE を書き込み可能な場所" +
              "（デスクトップやドキュメント、USB メモリなど）へ移してください。"
            : "既定では EXE と同じ場所の Data フォルダに保存します。EXE とこのフォルダを一緒にコピーすれば、" +
              "別のPCでもそのまま同じ内容で使えます。保存場所を変更した場合は次回起動時から有効になります" +
              "（現在のデータは自動では移動しません）。";

        var version = typeof(SettingsWindow).Assembly.GetName().Version?.ToString(3) ?? "1.0.0";
        VersionText.Text = $"お気に入りショートカット  バージョン {version}";
        StatsText.Text =
            $"ショートカット {_app.Store.Shortcuts.Count} 件 / " +
            $"フォルダ {_app.Store.AllFolders.Count} 件 / " +
            $"タグ {_app.Store.AllTagNames.Count()} 件";
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

    /// <summary>テーマだけは選んだ瞬間に反映して、見た目を確認できるようにする。</summary>
    private void OnThemeChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!_loaded) return;
        _app.ApplyTheme(ThemeCombo.SelectedIndex == 1 ? AppTheme.Dark : AppTheme.Light);
    }

    // ------------------------------------------------------------- ホットキー

    private void OnCaptureHotKey(object sender, RoutedEventArgs e)
    {
        if (_capturing)
        {
            StopCapturing();
            return;
        }

        _capturing = true;
        CaptureButton.Content = "キャンセル";
        HotKeyText.Text = "キーを押してください...";
        HotKeyStatus.Text = "Ctrl / Shift / Alt / Win と組み合わせたキーを押してください（Esc で中止）。";
        Keyboard.Focus(this);
    }

    private void StopCapturing()
    {
        _capturing = false;
        CaptureButton.Content = "変更";
        UpdateHotKeyDisplay();
    }

    protected override void OnPreviewKeyDown(KeyEventArgs e)
    {
        if (!_capturing)
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
        if (modifiers == ModifierKeys.None)
        {
            HotKeyStatus.Text = "修飾キー（Ctrl / Shift / Alt / Win）と組み合わせて押してください。";
            return;
        }

        _hotKeyModifiers = modifiers;
        _hotKeyKey = key;
        StopCapturing();
    }

    private void UpdateHotKeyDisplay()
    {
        HotKeyText.Text = HotKeyService.Describe(_hotKeyModifiers, _hotKeyKey);

        var current = _app.HotKeys.Current;
        var unchanged = current is not null &&
                        current.Value.Modifiers == _hotKeyModifiers &&
                        current.Value.Key == _hotKeyKey;

        HotKeyStatus.Text = _app.HotKeyRegistrationFailed && unchanged
            ? "このキーは他のアプリまたは Windows が使用しているため登録できませんでした。別のキーに変更してください。"
            : "他のアプリと競合する場合は、別の組み合わせに変更してください。";
    }

    // ------------------------------------------------------------- アイコン

    /// <summary>この PC で参照できるブラウザを表示する（設定の効果を分かりやすくするため）。</summary>
    private static string DescribeDetectedBrowsers()
    {
        var browsers = BrowserFaviconCache.DetectAvailableBrowsers();
        return browsers.Count == 0
            ? "このPCでは対象のブラウザが見つかりませんでした。"
            : "見つかったブラウザ: " + string.Join("、", browsers);
    }

    private void OnCleanIcons(object sender, RoutedEventArgs e)
    {
        var removed = _app.Icons.CleanUpUnusedIcons();
        CleanIconsResult.Text = removed == 0
            ? "整理の必要はありませんでした。"
            : $"{removed} 個のアイコンファイルを削除しました。";
    }

    // ------------------------------------------------------------- データ

    private void OnOpenDataFolder(object sender, RoutedEventArgs e) =>
        LaunchService.OpenPath(AppPaths.DataDirectory);

    private void OnOpenBackupFolder(object sender, RoutedEventArgs e) =>
        LaunchService.OpenPath(AppPaths.BackupDirectory);

    private void OnOpenLog(object sender, RoutedEventArgs e)
    {
        if (System.IO.File.Exists(AppPaths.LogFile)) LaunchService.OpenPath(AppPaths.LogFile);
        else MessageBox.Show(this, "ログファイルはまだありません。", "ログ",
            MessageBoxButton.OK, MessageBoxImage.Information);
    }

    private void OnChangeDataFolder(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog
        {
            Title = "データの保存場所を選択",
            InitialDirectory = AppPaths.DataDirectory,
        };
        if (dialog.ShowDialog(this) != true) return;

        _pendingDataDirectory = dialog.FolderName;
        _resetDataDirectory = false;
        DataPathText.Text = $"{AppPaths.DataDirectory}\n→ 次回起動から: {_pendingDataDirectory}";
    }

    private void OnResetDataFolder(object sender, RoutedEventArgs e)
    {
        _pendingDataDirectory = null;
        _resetDataDirectory = true;
        DataPathText.Text = $"{AppPaths.DataDirectory}\n→ 次回起動から: {AppPaths.DefaultDataDirectory}（既定）";
    }

    // ---------------------------------------------------------------- 保存

    private void OnSave(object sender, RoutedEventArgs e)
    {
        var s = _settings.Current.Clone();

        s.Theme = ThemeCombo.SelectedIndex == 1 ? AppTheme.Dark : AppTheme.Light;
        s.ViewMode = ViewModeCombo.SelectedIndex == 1 ? ShortcutViewMode.List : ShortcutViewMode.Card;
        s.IconSize = TagValue(IconSizeCombo, 32);
        s.LauncherMaxResults = TagValue(MaxResultsCombo, 12);
        s.HotKeyModifiers = (int)_hotKeyModifiers;
        s.HotKeyKey = (int)_hotKeyKey;
        s.ShowRecentInLauncher = ShowRecentCheck.IsChecked == true;
        s.MinimizeToTray = MinimizeToTrayCheck.IsChecked == true;
        s.StartMinimized = StartMinimizedCheck.IsChecked == true;
        s.RestoreLastFolder = RestoreFolderCheck.IsChecked == true;
        s.UseBrowserIconCache = BrowserIconCacheCheck.IsChecked == true;
        s.RunAtStartup = RunAtStartupCheck.IsChecked == true;

        _settings.Save(s);
        _app.ApplyTheme(s.Theme);

        if (!_app.RegisterHotKeyFromSettings())
        {
            MessageBox.Show(this,
                $"ホットキー {HotKeyService.Describe(_hotKeyModifiers, _hotKeyKey)} を登録できませんでした。\n\n" +
                "他のアプリまたは Windows が同じキーを使用している可能性があります。" +
                "設定画面から別の組み合わせに変更してください。",
                "ホットキー", MessageBoxButton.OK, MessageBoxImage.Warning);
        }

        if (StartupService.IsEnabled() != s.RunAtStartup && !StartupService.SetEnabled(s.RunAtStartup))
        {
            MessageBox.Show(this, "自動起動の設定を変更できませんでした。",
                "自動起動", MessageBoxButton.OK, MessageBoxImage.Warning);
        }

        if (_resetDataDirectory || _pendingDataDirectory is not null)
        {
            try
            {
                AppPaths.SaveDataDirectoryOverride(_resetDataDirectory ? null : _pendingDataDirectory);
                MessageBox.Show(this,
                    "データの保存場所を変更しました。次回このアプリを起動したときから有効になります。\n\n" +
                    "現在のデータは自動では移動しません。必要であれば、いったんエクスポートしてから" +
                    "新しい場所でインポートしてください。",
                    "保存場所", MessageBoxButton.OK, MessageBoxImage.Information);
            }
            catch (Exception ex)
            {
                AppLog.Error("保存場所の変更に失敗しました。", ex);
                MessageBox.Show(this, $"保存場所を変更できませんでした。\n\n{ex.Message}",
                    "エラー", MessageBoxButton.OK, MessageBoxImage.Error);
            }
        }

        DialogResult = true;
    }

    protected override void OnClosed(EventArgs e)
    {
        // キャンセルで閉じたときは、プレビュー用に変えていたテーマを元に戻す
        if (DialogResult != true) _app.ApplyTheme(_settings.Current.Theme);
        base.OnClosed(e);
    }
}

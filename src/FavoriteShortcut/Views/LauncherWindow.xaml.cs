using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;

namespace FavoriteShortcut.Views;

/// <summary>
/// ランチャー（§52〜§65）。
///
/// 「Ctrl + Space → 検索 → Enter」の 3 ステップで目的のものを開くための画面。
/// 表示のたびに作り直すと遅いので、一度作ったウィンドウは閉じずに隠しておき、
/// 次回は Show するだけにしている（§59）。
/// </summary>
public partial class LauncherWindow : Window
{
    private readonly App _app = App.Instance;
    private readonly AppStore _store;
    private readonly SettingsService _settings;
    private readonly IconService _icons;

    private readonly ObservableCollection<ShortcutItem> _results = new();

    public LauncherWindow()
    {
        InitializeComponent();

        _store = _app.Store;
        _settings = _app.Settings;
        _icons = _app.Icons;

        ResultList.ItemsSource = _results;
    }

    // ------------------------------------------------------------- 表示制御

    public void ShowLauncher()
    {
        QueryBox.Text = string.Empty;
        UpdateResults();
        PositionOnActiveMonitor();

        Show();
        Activate();
        ForceForeground();

        QueryBox.Focus();
        Keyboard.Focus(QueryBox);
    }

    public void HideLauncher()
    {
        Hide();
        _results.Clear();
    }

    /// <summary>
    /// マウスカーソルのあるモニターの中央やや上に表示する（§65: マルチモニター対応）。
    /// </summary>
    private void PositionOnActiveMonitor()
    {
        try
        {
            var screen = System.Windows.Forms.Screen.FromPoint(System.Windows.Forms.Cursor.Position);
            var area = screen.WorkingArea;

            // WorkingArea は物理ピクセルなので、DPI に合わせて論理単位へ直す
            var dpi = VisualTreeHelper.GetDpi(this);
            var left = area.Left / dpi.DpiScaleX;
            var top = area.Top / dpi.DpiScaleY;
            var width = area.Width / dpi.DpiScaleX;
            var height = area.Height / dpi.DpiScaleY;

            UpdateLayout();
            var windowHeight = ActualHeight > 0 ? ActualHeight : 260;

            Left = left + (width - Width) / 2;
            Top = top + Math.Max(0, height * 0.22 - windowHeight * 0.15);
        }
        catch (Exception ex)
        {
            AppLog.Warn("ランチャーの表示位置を計算できませんでした。", ex);
            WindowStartupLocation = WindowStartupLocation.CenterScreen;
        }
    }

    /// <summary>
    /// 他のアプリを操作中にホットキーで呼ばれるため、Activate() だけでは前面に来ないことがある。
    /// Win32 で明示的に前面へ出す。
    /// </summary>
    private void ForceForeground()
    {
        try
        {
            var handle = new WindowInteropHelper(this).Handle;
            if (handle != IntPtr.Zero) SetForegroundWindow(handle);
        }
        catch (Exception ex)
        {
            AppLog.Warn("ランチャーを前面に出せませんでした。", ex);
        }
    }

    private void OnDeactivated(object? sender, EventArgs e)
    {
        // 別のウィンドウをクリックしたら閉じる（§64）
        if (IsVisible) HideLauncher();
    }

    protected override void OnClosing(CancelEventArgs e)
    {
        // アプリ終了時以外は破棄せず隠すだけにして、次回の表示を速くする
        if (!_app.IsExiting && !_app.Dispatcher.HasShutdownStarted)
        {
            e.Cancel = true;
            HideLauncher();
            return;
        }

        base.OnClosing(e);
    }

    // ------------------------------------------------------------- 検索

    private void OnQueryChanged(object sender, TextChangedEventArgs e)
    {
        QueryPlaceholder.Visibility = QueryBox.Text.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        UpdateResults();
    }

    private void UpdateResults()
    {
        var max = _settings.Current.LauncherMaxResults;
        var query = QueryBox.Text;

        List<ShortcutItem> items;
        string header;

        var hits = SearchService.Search(_store.Shortcuts, query, _store.Revision, max);
        if (hits is not null)
        {
            items = hits.Select(h => h.Item).ToList();
            header = $"検索結果（{items.Count} 件）";
        }
        else if (_settings.Current.ShowRecentInLauncher)
        {
            items = SearchService.Recent(_store.Shortcuts, max);
            header = items.Count > 0 ? "最近使った項目" : string.Empty;
        }
        else
        {
            items = new List<ShortcutItem>();
            header = string.Empty;
        }

        _results.Clear();
        foreach (var item in items)
        {
            _icons.Attach(item);
            _results.Add(item);
        }

        ResultHeader.Text = header;
        ResultHeader.Visibility = header.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        ResultList.Visibility = items.Count == 0 ? Visibility.Collapsed : Visibility.Visible;

        if (items.Count > 0) ResultList.SelectedIndex = 0;

        if (items.Count == 0)
        {
            NoResultText.Visibility = Visibility.Visible;
            NoResultText.Text = query.Trim().Length > 0
                ? "該当するショートカットがありません。タイトル・タグ・フォルダ名・URL / パスが検索対象です。"
                : _store.Shortcuts.Count == 0
                    ? "まだショートカットが登録されていません。管理画面から登録してください。"
                    : "キーワードを入力してください。";
        }
        else
        {
            NoResultText.Visibility = Visibility.Collapsed;
        }

        FooterRight.Text = $"全 {_store.Shortcuts.Count} 件";
    }

    // ------------------------------------------------------------- キー操作

    private void OnQueryKeyDown(object sender, KeyEventArgs e)
    {
        // IME 変換中のキーには手を出さない（変換のスペースや確定の Enter を奪わない）
        if (e.Key is Key.ImeProcessed or Key.DeadCharProcessed) return;

        switch (e.Key)
        {
            case Key.Escape:
                HideLauncher();
                e.Handled = true;
                break;

            case Key.Down:
                MoveSelection(1);
                e.Handled = true;
                break;

            case Key.Up:
                MoveSelection(-1);
                e.Handled = true;
                break;

            case Key.PageDown:
                MoveSelection(5);
                e.Handled = true;
                break;

            case Key.PageUp:
                MoveSelection(-5);
                e.Handled = true;
                break;

            case Key.Enter:
                LaunchSelected();
                e.Handled = true;
                break;

            case Key.Tab:
                MoveSelection(Keyboard.Modifiers == ModifierKeys.Shift ? -1 : 1);
                e.Handled = true;
                break;
        }
    }

    private void MoveSelection(int delta)
    {
        if (_results.Count == 0) return;

        var index = ResultList.SelectedIndex + delta;
        index = Math.Clamp(index, 0, _results.Count - 1);
        ResultList.SelectedIndex = index;
        ResultList.ScrollIntoView(ResultList.SelectedItem);
    }

    private void LaunchSelected()
    {
        if (ResultList.SelectedItem is not ShortcutItem item) return;

        // 先に閉じてから起動すると、開いたアプリが最前面に出て自然に見える
        HideLauncher();

        var result = LaunchService.Launch(item);
        if (result.Success)
        {
            _store.RecordUsage(item);
            _icons.ScheduleRecheckAfterLaunch(item);
            return;
        }

        MessageBox.Show(
            $"{result.ErrorTitle}\n\n{result.ErrorDetail}\n\n登録内容はそのまま残っています。",
            "お気に入りショートカット", MessageBoxButton.OK, MessageBoxImage.Warning);
    }

    private void OnResultClick(object sender, MouseButtonEventArgs e)
    {
        if (FindAncestor<ListBoxItem>((DependencyObject)e.OriginalSource) is not { } container) return;
        container.IsSelected = true;
        LaunchSelected();
        e.Handled = true;
    }

    private void OnResultRightClick(object sender, MouseButtonEventArgs e)
    {
        if (FindAncestor<ListBoxItem>((DependencyObject)e.OriginalSource) is not { } container) return;
        container.IsSelected = true;
    }

    // --------------------------------------------------------- コンテキスト

    private ShortcutItem? Selected => ResultList.SelectedItem as ShortcutItem;

    private void OnMenuOpen(object sender, RoutedEventArgs e) => LaunchSelected();

    private void OnMenuEdit(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } item) return;

        HideLauncher();
        _app.ShowMainWindow();

        var dialog = new ShortcutEditWindow(item) { Owner = Application.Current.MainWindow };
        dialog.ShowDialog();
    }

    private void OnMenuCopy(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } item) return;
        try { Clipboard.SetText(item.Target); }
        catch (Exception ex) { AppLog.Warn("クリップボードへのコピーに失敗しました。", ex); }
    }

    private void OnMenuReveal(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } item) return;

        HideLauncher();
        var result = LaunchService.RevealInExplorer(item);
        if (!result.Success)
        {
            MessageBox.Show($"{result.ErrorTitle}\n\n{result.ErrorDetail}",
                "お気に入りショートカット", MessageBoxButton.OK, MessageBoxImage.Warning);
        }
    }

    private void OnMenuDelete(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } item) return;

        var answer = MessageBox.Show(
            $"「{item.DisplayTitle}」を削除しますか？\n\nこの操作は元に戻せません。",
            "削除の確認", MessageBoxButton.OKCancel, MessageBoxImage.Warning);
        if (answer != MessageBoxResult.OK) return;

        _icons.DeleteCustomIcon(item.IconPath);
        _store.DeleteShortcut(item);
        UpdateResults();
    }

    private static T? FindAncestor<T>(DependencyObject? current) where T : DependencyObject
    {
        while (current is not null)
        {
            if (current is T typed) return typed;
            current = VisualTreeHelper.GetParent(current);
        }
        return null;
    }

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetForegroundWindow(IntPtr hWnd);
}

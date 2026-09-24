using System.Collections.ObjectModel;
using System.ComponentModel;
using System.IO;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;
using Microsoft.Win32;

namespace FavoriteShortcut.Views;

public partial class MainWindow : Window, INotifyPropertyChanged
{
    /// <summary>ショートカットのドラッグ＆ドロップで使う独自フォーマット。</summary>
    private const string ShortcutDragFormat = "FavoriteShortcut.ShortcutIds";
    private const string FolderDragFormat = "FavoriteShortcut.FolderId";

    /// <summary>
    /// カード表示は WrapPanel のため仮想化が効かない。
    /// 大量登録時に固まらないよう表示件数を打ち切る（打ち切った場合は画面で通知する）。
    /// リスト表示は仮想化されるので上限なし。
    /// </summary>
    private const int MaxCardItems = 1000;

    private readonly App _app = App.Instance;
    private readonly AppStore _store;
    private readonly SettingsService _settings;
    private readonly IconService _icons;

    private readonly SpecialFolderNode _allNode = new(SpecialFolderKind.All, "すべてのショートカット", "★");
    private readonly SpecialFolderNode _uncategorizedNode = new(SpecialFolderKind.Uncategorized, "未分類", "◇");

    private readonly ObservableCollection<ShortcutItem> _displayed = new();
    private readonly DispatcherTimer _searchTimer;

    private object? _scope;
    private int _iconSize = 32;
    private Point _dragStart;
    private bool _suppressRefresh;

    public MainWindow()
    {
        InitializeComponent();

        _store = _app.Store;
        _settings = _app.Settings;
        _icons = _app.Icons;

        _searchTimer = new DispatcherTimer(DispatcherPriority.Background)
        {
            Interval = TimeSpan.FromMilliseconds(140),
        };
        _searchTimer.Tick += (_, _) =>
        {
            _searchTimer.Stop();
            RefreshList();
        };

        ShortcutList.ItemsSource = _displayed;
        BuildFolderTree();

        _store.DataChanged += OnStoreChanged;
        _settings.Changed += (_, _) => ApplySettings();

        ApplySettings();
        RestoreScope();
        RefreshTagCloud();
        RefreshList();

        Loaded += (_, _) => SearchBox.Focus();
    }

    // ------------------------------------------------------------- プロパティ

    /// <summary>一覧のアイコン表示サイズ。DataTemplate から参照される。</summary>
    public int IconSize
    {
        get => _iconSize;
        set
        {
            if (_iconSize == value) return;
            _iconSize = value;
            OnPropertyChanged();
        }
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    private void OnPropertyChanged([CallerMemberName] string? name = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));

    // ----------------------------------------------------------------- 設定

    private void ApplySettings()
    {
        var s = _settings.Current;

        IconSize = s.IconSize;
        Width = Math.Max(MinWidth, s.MainWindowWidth);
        Height = Math.Max(MinHeight, s.MainWindowHeight);
        FolderPaneColumn.Width = new GridLength(Math.Max(170, s.FolderPaneWidth));

        SetViewMode(s.ViewMode, save: false);
        UpdateHotKeyHint();
    }

    private void UpdateHotKeyHint()
    {
        var current = _app.HotKeys.Current;
        HotKeyHint.Text = current is null
            ? "ランチャーのホットキーは未登録です（設定で変更できます）"
            : $"ランチャー: {HotKeyService.Describe(current.Value.Modifiers, current.Value.Key)}";
    }

    public void PersistWindowState()
    {
        try
        {
            var s = _settings.Current.Clone();
            if (WindowState == WindowState.Normal)
            {
                s.MainWindowWidth = Width;
                s.MainWindowHeight = Height;
            }
            s.FolderPaneWidth = FolderPaneColumn.ActualWidth;
            s.LastFolderId = _scope is FolderItem f ? f.Id : null;
            _settings.Save(s);
        }
        catch (Exception ex)
        {
            AppLog.Warn("ウィンドウ状態の保存に失敗しました。", ex);
        }
    }

    protected override void OnClosing(CancelEventArgs e)
    {
        // 終了処理から閉じられた場合は、何もせずそのまま閉じる
        if (_app.IsExiting)
        {
            _store.DataChanged -= OnStoreChanged;
            base.OnClosing(e);
            return;
        }

        PersistWindowState();

        // 閉じるボタンではアプリを終了せず、トレイに常駐させる（§67）
        if (_app.ShouldStayInTray)
        {
            e.Cancel = true;
            Hide();
            return;
        }

        _store.DataChanged -= OnStoreChanged;
        base.OnClosing(e);
        _app.ExitApplication();
    }

    // -------------------------------------------------------------- ツリー

    private void BuildFolderTree()
    {
        var composite = new CompositeCollection
        {
            _allNode,
            _uncategorizedNode,
            new CollectionContainer { Collection = _store.RootFolders },
        };
        FolderTree.ItemsSource = composite;
    }

    private void RestoreScope()
    {
        _scope = _allNode;

        if (_settings.Current.RestoreLastFolder && _settings.Current.LastFolderId is { } id)
        {
            var folder = _store.FindFolder(id);
            if (folder is not null) _scope = folder;
        }

        SelectScopeInTree();
    }

    private void SelectScopeInTree()
    {
        _suppressRefresh = true;
        try
        {
            if (_scope is not null && TryFindTreeItem(FolderTree, _scope) is { } container)
            {
                container.IsSelected = true;
                container.BringIntoView();
            }
        }
        finally
        {
            _suppressRefresh = false;
        }
    }

    private static TreeViewItem? TryFindTreeItem(ItemsControl parent, object target)
    {
        foreach (var item in parent.Items)
        {
            if (parent.ItemContainerGenerator.ContainerFromItem(item) is not TreeViewItem container) continue;
            if (ReferenceEquals(item, target)) return container;

            container.UpdateLayout();
            var found = TryFindTreeItem(container, target);
            if (found is not null) return found;
        }
        return null;
    }

    private void OnFolderSelectionChanged(object sender, RoutedPropertyChangedEventArgs<object> e)
    {
        if (_suppressRefresh) return;
        _scope = e.NewValue;
        RefreshList();
    }

    private void OnStoreChanged(object? sender, EventArgs e)
    {
        // 選択していたフォルダが消えていたら「すべて」に戻す
        if (_scope is FolderItem f && _store.FindFolder(f.Id) is null) _scope = _allNode;

        RefreshTagCloud();
        RefreshList();
    }

    // -------------------------------------------------------------- 一覧更新

    private void RefreshList()
    {
        var query = SearchBox.Text;
        var source = ScopeItems().ToList();

        List<ShortcutItem> result;
        var hits = SearchService.Search(source, query, _store.Revision);
        if (hits is not null)
        {
            result = hits.Select(h => h.Item).ToList();
        }
        else
        {
            result = source
                .OrderBy(s => s.SortOrder)
                .ThenBy(s => s.DisplayTitle, StringComparer.CurrentCultureIgnoreCase)
                .ToList();
        }

        var truncated = false;
        if (_settings.Current.ViewMode == ShortcutViewMode.Card && result.Count > MaxCardItems)
        {
            truncated = true;
            result = result.Take(MaxCardItems).ToList();
        }

        _displayed.Clear();
        foreach (var item in result)
        {
            _icons.Attach(item);
            _displayed.Add(item);
        }

        UpdateHeaderTexts(source.Count, result.Count, truncated, !string.IsNullOrWhiteSpace(query));
    }

    private IEnumerable<ShortcutItem> ScopeItems()
    {
        switch (_scope)
        {
            case FolderItem folder:
            {
                // サブフォルダの中身も含めて表示する（親フォルダを選ぶと全体が見える）
                var ids = _store.CollectFolderAndDescendants(folder)
                    .Select(f => f.Id).ToHashSet(StringComparer.Ordinal);
                return _store.Shortcuts.Where(s => s.FolderId is not null && ids.Contains(s.FolderId));
            }

            case SpecialFolderNode { Kind: SpecialFolderKind.Uncategorized }:
                return _store.Shortcuts.Where(s => s.FolderId is null);

            default:
                return _store.Shortcuts;
        }
    }

    private void UpdateHeaderTexts(int scopeCount, int shownCount, bool truncated, bool searching)
    {
        ScopeText.Text = _scope switch
        {
            FolderItem f => f.FullPath,
            SpecialFolderNode { Kind: SpecialFolderKind.Uncategorized } => "未分類",
            _ => "すべてのショートカット",
        };

        CountText.Text = searching
            ? $"{shownCount} 件ヒット / このフォルダ {scopeCount} 件"
            : $"{shownCount} 件";

        TruncationNotice.Visibility = truncated ? Visibility.Visible : Visibility.Collapsed;
        if (truncated)
        {
            TruncationText.Text =
                $"表示が重くならないよう、カード表示では先頭 {MaxCardItems} 件のみ表示しています。" +
                "検索で絞り込むか、リスト表示に切り替えるとすべて表示できます。";
        }

        var empty = shownCount == 0;
        EmptyState.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
        ShortcutList.Visibility = empty ? Visibility.Collapsed : Visibility.Visible;

        if (empty)
        {
            EmptyTitle.Text = searching ? "見つかりませんでした" : "ショートカットがありません";
            EmptyHint.Text = searching
                ? "別のキーワードをお試しください。タイトル・タグ・フォルダ名・URL / パスが検索対象です。"
                : "「＋ 新規登録」から追加するか、エクスプローラーからファイル・フォルダをここへドラッグしてください。";
        }

        StatusText.Text = $"全 {_store.Shortcuts.Count} 件 / フォルダ {_store.AllFolders.Count} 件 / タグ {_store.AllTagNames.Count()} 件";
    }

    private void RefreshTagCloud()
    {
        TagCloud.Items.Clear();
        var tags = _store.GetTagUsage();

        NoTagsText.Visibility = tags.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

        foreach (var (name, count) in tags.Take(60))
        {
            var button = new Button
            {
                Content = $"{name}  {count}",
                Style = (Style)FindResource("SecondaryButton"),
                Margin = new Thickness(0, 0, 5, 5),
                Padding = new Thickness(9, 3, 9, 3),
                FontSize = 11.5,
                Tag = name,
                ToolTip = $"「{name}」で検索",
            };
            button.Click += (_, _) =>
            {
                SearchBox.Text = name;
                SearchBox.CaretIndex = SearchBox.Text.Length;
                SearchBox.Focus();
            };
            TagCloud.Items.Add(button);
        }
    }

    // ---------------------------------------------------------------- 検索

    private void OnSearchTextChanged(object sender, TextChangedEventArgs e)
    {
        SearchPlaceholder.Visibility = SearchBox.Text.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        ClearSearchButton.Visibility = SearchBox.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;

        _searchTimer.Stop();
        _searchTimer.Start();
    }

    private void OnSearchKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key is Key.ImeProcessed or Key.DeadCharProcessed) return;

        switch (e.Key)
        {
            case Key.Escape:
                if (SearchBox.Text.Length > 0)
                {
                    SearchBox.Clear();
                    e.Handled = true;
                }
                break;

            case Key.Down:
            case Key.Enter:
                if (_displayed.Count > 0)
                {
                    ShortcutList.Focus();
                    ShortcutList.SelectedIndex = 0;
                    if (ShortcutList.ItemContainerGenerator.ContainerFromIndex(0) is ListBoxItem lbi)
                        lbi.Focus();
                    if (e.Key == Key.Enter) OpenSelected();
                    e.Handled = true;
                }
                break;
        }
    }

    private void OnClearSearch(object sender, RoutedEventArgs e)
    {
        SearchBox.Clear();
        SearchBox.Focus();
    }

    private void OnFocusSearch(object sender, ExecutedRoutedEventArgs e)
    {
        SearchBox.Focus();
        SearchBox.SelectAll();
    }

    // -------------------------------------------------------------- 表示切替

    private void OnViewModeChanged(object sender, RoutedEventArgs e)
    {
        var mode = ReferenceEquals(sender, ListViewToggle) ? ShortcutViewMode.List : ShortcutViewMode.Card;
        SetViewMode(mode, save: true);
        RefreshList();
    }

    private void SetViewMode(ShortcutViewMode mode, bool save)
    {
        CardViewToggle.IsChecked = mode == ShortcutViewMode.Card;
        ListViewToggle.IsChecked = mode == ShortcutViewMode.List;

        ShortcutList.ItemTemplate = (DataTemplate)FindResource(
            mode == ShortcutViewMode.Card ? "CardTemplate" : "ListTemplate");
        ShortcutList.ItemsPanel = (ItemsPanelTemplate)FindResource(
            mode == ShortcutViewMode.Card ? "CardPanel" : "ListPanel");

        if (!save || _settings.Current.ViewMode == mode) return;

        var s = _settings.Current.Clone();
        s.ViewMode = mode;
        _settings.Save(s);
    }

    // ------------------------------------------------------------ 起動/操作

    private void OnShortcutDoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (FindAncestor<ListBoxItem>((DependencyObject)e.OriginalSource) is null) return;
        OpenSelected();
    }

    private void OnShortcutListKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key is Key.ImeProcessed or Key.DeadCharProcessed) return;

        switch (e.Key)
        {
            case Key.Enter:
                OpenSelected();
                e.Handled = true;
                break;

            case Key.Delete:
                DeleteSelected();
                e.Handled = true;
                break;

            case Key.F2:
                EditSelected();
                e.Handled = true;
                break;

            case Key.C when Keyboard.Modifiers == ModifierKeys.Control:
                CopySelectedTarget();
                e.Handled = true;
                break;
        }
    }

    private void OpenSelected()
    {
        var items = SelectedShortcuts();
        if (items.Count == 0) return;

        foreach (var item in items)
        {
            var result = LaunchService.Launch(item);
            if (result.Success)
            {
                _store.RecordUsage(item);

                // ブラウザで開くと favicon がブラウザ側にキャッシュされるので、
                // 少し待ってから取り直す（一度開けばアイコンが付く）
                _icons.ScheduleRecheckAfterLaunch(item);
            }
            else
            {
                MessageBox.Show(this,
                    $"{result.ErrorTitle}\n\n{result.ErrorDetail}\n\n" +
                    "登録内容は残っています。編集画面からパスを修正できます。",
                    "お気に入りショートカット", MessageBoxButton.OK, MessageBoxImage.Warning);
            }
        }
    }

    private List<ShortcutItem> SelectedShortcuts() =>
        ShortcutList.SelectedItems.Cast<ShortcutItem>().ToList();

    private void OnOpenShortcut(object sender, RoutedEventArgs e) => OpenSelected();

    private void OnEditShortcut(object sender, RoutedEventArgs e) => EditSelected();

    private void EditSelected()
    {
        if (ShortcutList.SelectedItem is not ShortcutItem item) return;

        var dialog = new ShortcutEditWindow(item) { Owner = this };
        if (dialog.ShowDialog() == true) RefreshList();
    }

    private void OnNewShortcut(object sender, RoutedEventArgs e) => CreateShortcut(null);

    private void OnNewShortcut(object sender, ExecutedRoutedEventArgs e) => CreateShortcut(null);

    private void CreateShortcut(string? initialTarget)
    {
        var folderId = _scope is FolderItem f ? f.Id : null;
        var dialog = new ShortcutEditWindow(null, folderId, initialTarget) { Owner = this };
        if (dialog.ShowDialog() == true) RefreshList();
    }

    private void OnDuplicateShortcut(object sender, RoutedEventArgs e)
    {
        if (ShortcutList.SelectedItem is not ShortcutItem item) return;

        var copy = item.Clone();
        copy.Id = Guid.NewGuid().ToString("N");
        copy.Title = item.Title + " のコピー";
        copy.UsageCount = 0;
        copy.LastUsedAt = null;
        _store.AddShortcut(copy);
        RefreshList();
    }

    private void OnCopyTarget(object sender, RoutedEventArgs e) => CopySelectedTarget();

    private void CopySelectedTarget()
    {
        var items = SelectedShortcuts();
        if (items.Count == 0) return;
        SetClipboard(string.Join(Environment.NewLine, items.Select(i => i.Target)));
    }

    private void OnCopyTitle(object sender, RoutedEventArgs e)
    {
        var items = SelectedShortcuts();
        if (items.Count == 0) return;
        SetClipboard(string.Join(Environment.NewLine, items.Select(i => i.DisplayTitle)));
    }

    private void SetClipboard(string text)
    {
        try
        {
            Clipboard.SetText(text);
            StatusText.Text = "クリップボードにコピーしました。";
        }
        catch (Exception ex)
        {
            AppLog.Warn("クリップボードへのコピーに失敗しました。", ex);
        }
    }

    private void OnRevealInExplorer(object sender, RoutedEventArgs e)
    {
        if (ShortcutList.SelectedItem is not ShortcutItem item) return;

        var result = LaunchService.RevealInExplorer(item);
        if (!result.Success)
        {
            MessageBox.Show(this, $"{result.ErrorTitle}\n\n{result.ErrorDetail}",
                "お気に入りショートカット", MessageBoxButton.OK, MessageBoxImage.Warning);
        }
    }

    private async void OnRefreshIcon(object sender, RoutedEventArgs e)
    {
        foreach (var item in SelectedShortcuts())
        {
            if (item.TargetType == TargetType.Web)
            {
                await _icons.EnsureFaviconAsync(item, force: true);
            }
            else
            {
                _icons.ClearMemoryCache();
                _icons.Attach(item);
            }
        }
    }

    private void OnMoveToFolder(object sender, RoutedEventArgs e)
    {
        var items = SelectedShortcuts();
        if (items.Count == 0) return;

        var dialog = new FolderPickerWindow(_store, items[0].FolderId) { Owner = this };
        if (dialog.ShowDialog() != true) return;

        foreach (var item in items) _store.MoveShortcut(item, dialog.SelectedFolderId);
        RefreshList();
    }

    private void OnDeleteShortcut(object sender, RoutedEventArgs e) => DeleteSelected();

    private void DeleteSelected()
    {
        var items = SelectedShortcuts();
        if (items.Count == 0) return;

        var message = items.Count == 1
            ? $"「{items[0].DisplayTitle}」を削除しますか？"
            : $"選択した {items.Count} 件のショートカットを削除しますか？";

        if (MessageBox.Show(this, message + "\n\nこの操作は元に戻せません。", "削除の確認",
                MessageBoxButton.OKCancel, MessageBoxImage.Warning) != MessageBoxResult.OK)
            return;

        foreach (var item in items)
        {
            _icons.DeleteCustomIcon(item.IconPath);
            _store.DeleteShortcut(item);
        }

        RefreshList();
    }

    private void OnListRightButtonDown(object sender, MouseButtonEventArgs e)
    {
        if (FindAncestor<ListBoxItem>((DependencyObject)e.OriginalSource) is not { } container) return;

        // 右クリックした項目が未選択なら、その項目だけを選択してからメニューを出す
        if (!container.IsSelected)
        {
            ShortcutList.SelectedItems.Clear();
            container.IsSelected = true;
        }
    }

    // -------------------------------------------------------------- フォルダ

    private void OnNewFolder(object sender, RoutedEventArgs e) => CreateFolder(null);

    private void OnNewSubFolder(object sender, RoutedEventArgs e) =>
        CreateFolder(_scope is FolderItem f ? f.Id : null);

    private void CreateFolder(string? parentId)
    {
        var dialog = new TextInputWindow("新しいフォルダ", "フォルダ名", string.Empty) { Owner = this };
        if (dialog.ShowDialog() != true) return;

        var created = _store.CreateFolder(dialog.Value, parentId);
        _scope = created;
        SelectScopeInTree();
        RefreshList();
    }

    private void OnRenameFolder(object sender, RoutedEventArgs e)
    {
        if (_scope is not FolderItem folder) return;

        var dialog = new TextInputWindow("フォルダ名の変更", "フォルダ名", folder.Name) { Owner = this };
        if (dialog.ShowDialog() != true) return;

        _store.RenameFolder(folder, dialog.Value);
        RefreshList();
    }

    private void OnDeleteFolder(object sender, RoutedEventArgs e)
    {
        if (_scope is not FolderItem folder) return;

        var descendants = _store.CollectFolderAndDescendants(folder);
        var ids = descendants.Select(f => f.Id).ToHashSet(StringComparer.Ordinal);
        var count = _store.Shortcuts.Count(s => s.FolderId is not null && ids.Contains(s.FolderId));

        var dialog = new FolderDeleteWindow(folder.Name, descendants.Count - 1, count) { Owner = this };
        if (dialog.ShowDialog() != true) return;

        _store.DeleteFolder(folder, dialog.DeleteShortcuts);
        _scope = _allNode;
        SelectScopeInTree();
        RefreshList();
    }

    private void OnTreeRightButtonDown(object sender, MouseButtonEventArgs e)
    {
        var container = FindAncestor<TreeViewItem>((DependencyObject)e.OriginalSource);
        if (container is not null)
        {
            container.IsSelected = true;
            container.Focus();
        }

        var isRealFolder = _scope is FolderItem;
        TreeRenameItem.IsEnabled = isRealFolder;
        TreeDeleteItem.IsEnabled = isRealFolder;
        TreeNewSubFolderItem.IsEnabled = isRealFolder;
    }

    private void OnManageTags(object sender, RoutedEventArgs e)
    {
        var dialog = new TagManagerWindow(_store) { Owner = this };
        dialog.ShowDialog();
        RefreshTagCloud();
        RefreshList();
    }

    // ------------------------------------------------------- ドラッグ＆ドロップ

    private void OnListPreviewMouseDown(object sender, MouseButtonEventArgs e) =>
        _dragStart = e.GetPosition(null);

    private void OnListPreviewMouseMove(object sender, MouseEventArgs e)
    {
        if (e.LeftButton != MouseButtonState.Pressed) return;
        if (!ExceededDragThreshold(e.GetPosition(null))) return;
        if (FindAncestor<ListBoxItem>((DependencyObject)e.OriginalSource) is null) return;

        var ids = SelectedShortcuts().Select(s => s.Id).ToArray();
        if (ids.Length == 0) return;

        var data = new DataObject(ShortcutDragFormat, ids);
        DragDrop.DoDragDrop(ShortcutList, data, DragDropEffects.Move);
    }

    private void OnTreePreviewMouseDown(object sender, MouseButtonEventArgs e) =>
        _dragStart = e.GetPosition(null);

    private void OnTreePreviewMouseMove(object sender, MouseEventArgs e)
    {
        if (e.LeftButton != MouseButtonState.Pressed) return;
        if (!ExceededDragThreshold(e.GetPosition(null))) return;

        var container = FindAncestor<TreeViewItem>((DependencyObject)e.OriginalSource);
        if (container?.DataContext is not FolderItem folder) return;

        var data = new DataObject(FolderDragFormat, folder.Id);
        DragDrop.DoDragDrop(FolderTree, data, DragDropEffects.Move);
    }

    private bool ExceededDragThreshold(Point current) =>
        Math.Abs(current.X - _dragStart.X) > SystemParameters.MinimumHorizontalDragDistance ||
        Math.Abs(current.Y - _dragStart.Y) > SystemParameters.MinimumVerticalDragDistance;

    private void OnTreeDragOver(object sender, DragEventArgs e)
    {
        e.Effects = DragDropEffects.None;

        var target = DropTargetOf(e);
        if (target is null && !e.Data.GetDataPresent(DataFormats.FileDrop)) { e.Handled = true; return; }

        if (e.Data.GetDataPresent(ShortcutDragFormat)) e.Effects = DragDropEffects.Move;
        else if (e.Data.GetDataPresent(FolderDragFormat)) e.Effects = DragDropEffects.Move;
        else if (e.Data.GetDataPresent(DataFormats.FileDrop)) e.Effects = DragDropEffects.Copy;

        e.Handled = true;
    }

    private void OnTreeDrop(object sender, DragEventArgs e)
    {
        e.Handled = true;
        var target = DropTargetOf(e);

        if (e.Data.GetDataPresent(ShortcutDragFormat))
        {
            if (target is not { } dropTarget) return;
            var ids = (string[])e.Data.GetData(ShortcutDragFormat)!;
            foreach (var id in ids)
                if (_store.FindShortcut(id) is { } item)
                    _store.MoveShortcut(item, dropTarget.FolderId);
            RefreshList();
            return;
        }

        if (e.Data.GetDataPresent(FolderDragFormat))
        {
            if (target is not { } dropTarget) return;
            var id = (string)e.Data.GetData(FolderDragFormat)!;
            if (_store.FindFolder(id) is not { } folder) return;

            if (!_store.MoveFolder(folder, dropTarget.FolderId))
            {
                MessageBox.Show(this, "フォルダを自分自身やその配下へは移動できません。",
                    "お気に入りショートカット", MessageBoxButton.OK, MessageBoxImage.Information);
            }
            RefreshList();
            return;
        }

        if (e.Data.GetDataPresent(DataFormats.FileDrop))
        {
            var paths = (string[])e.Data.GetData(DataFormats.FileDrop)!;
            ImportDroppedPaths(paths, target?.FolderId);
        }
    }

    /// <summary>ドロップ先のフォルダ。「未分類」「すべて」にドロップした場合は FolderId が null になる。</summary>
    private (string? FolderId, object Node)? DropTargetOf(DragEventArgs e)
    {
        var container = FindAncestor<TreeViewItem>((DependencyObject)e.OriginalSource);
        return container?.DataContext switch
        {
            FolderItem folder => (folder.Id, folder),
            SpecialFolderNode node => ((string?)null, node),
            _ => null,
        };
    }

    private void OnListDragOver(object sender, DragEventArgs e)
    {
        e.Effects = e.Data.GetDataPresent(DataFormats.FileDrop) || e.Data.GetDataPresent(DataFormats.UnicodeText)
            ? DragDropEffects.Copy
            : DragDropEffects.None;
        e.Handled = true;
    }

    private void OnListDrop(object sender, DragEventArgs e)
    {
        e.Handled = true;
        var folderId = _scope is FolderItem f ? f.Id : null;

        if (e.Data.GetDataPresent(DataFormats.FileDrop))
        {
            ImportDroppedPaths((string[])e.Data.GetData(DataFormats.FileDrop)!, folderId);
            return;
        }

        if (e.Data.GetDataPresent(DataFormats.UnicodeText))
        {
            var text = ((string)e.Data.GetData(DataFormats.UnicodeText)!).Trim();
            if (text.Length == 0) return;

            var dialog = new ShortcutEditWindow(null, folderId, text) { Owner = this };
            if (dialog.ShowDialog() == true) RefreshList();
        }
    }

    /// <summary>エクスプローラーからドロップされたファイル/フォルダをショートカットとして登録する（§18）。</summary>
    private void ImportDroppedPaths(IReadOnlyList<string> paths, string? folderId)
    {
        if (paths.Count == 0) return;

        // 1 件だけなら内容を確認・編集できるようダイアログを出す
        if (paths.Count == 1)
        {
            var dialog = new ShortcutEditWindow(null, folderId, paths[0]) { Owner = this };
            if (dialog.ShowDialog() == true) RefreshList();
            return;
        }

        var message = $"{paths.Count} 件のファイル / フォルダをショートカットとして登録しますか？";
        if (MessageBox.Show(this, message, "まとめて登録", MessageBoxButton.OKCancel,
                MessageBoxImage.Question) != MessageBoxResult.OK)
            return;

        foreach (var path in paths)
        {
            var type = TargetResolver.Detect(path);
            _store.AddShortcut(new ShortcutItem
            {
                Title = TargetResolver.SuggestTitle(path, type),
                Target = path,
                TargetType = type,
                FolderId = folderId,
            });
        }

        RefreshList();
    }

    // ------------------------------------------------------------- メニュー

    private void OnMenuButtonClick(object sender, RoutedEventArgs e)
    {
        if (sender is not Button button || button.ContextMenu is null) return;
        button.ContextMenu.PlacementTarget = button;
        button.ContextMenu.Placement = System.Windows.Controls.Primitives.PlacementMode.Bottom;
        button.ContextMenu.IsOpen = true;
    }

    public void OpenSettings()
    {
        var dialog = new SettingsWindow() { Owner = this };
        dialog.ShowDialog();
        ApplySettings();
        RefreshList();
    }

    private void OnOpenSettings(object sender, RoutedEventArgs e) => OpenSettings();

    private void OnShowLauncher(object sender, RoutedEventArgs e) => _app.ShowLauncher();

    /// <summary>Edge / Chrome などのお気に入りを取り込む。</summary>
    private void OnImportBookmarks(object sender, RoutedEventArgs e)
    {
        var folderId = _scope is FolderItem f ? f.Id : null;
        var dialog = new BookmarkImportWindow(_store, _icons, folderId) { Owner = this };
        if (dialog.ShowDialog() != true) return;

        BuildFolderTree();
        SelectScopeInTree();
        RefreshTagCloud();
        RefreshList();

        var skipped = dialog.SkippedCount > 0
            ? $"\n重複のためスキップ: {dialog.SkippedCount} 件"
            : string.Empty;

        MessageBox.Show(this,
            $"お気に入りを取り込みました。\n\n" +
            $"取り込んだショートカット: {dialog.ImportedCount} 件{skipped}\n\n" +
            "アイコンは順次取得されます。",
            "取り込み完了", MessageBoxButton.OK, MessageBoxImage.Information);
    }

    private void OnExport(object sender, RoutedEventArgs e)
    {
        var dialog = new SaveFileDialog
        {
            Title = "エクスポート先を選択",
            Filter = "お気に入りショートカット データ (*.zip)|*.zip",
            FileName = ExportImportService.SuggestedFileName,
            AddExtension = true,
        };
        if (dialog.ShowDialog(this) != true) return;

        try
        {
            Mouse.OverrideCursor = Cursors.Wait;
            _app.Transfer.Export(dialog.FileName);
            MessageBox.Show(this,
                $"エクスポートしました。\n\n{dialog.FileName}\n\n" +
                "このファイルを別のPCでインポートすると、フォルダ・タグ・アイコンごと復元できます。",
                "エクスポート完了", MessageBoxButton.OK, MessageBoxImage.Information);
        }
        catch (Exception ex)
        {
            AppLog.Error("エクスポートに失敗しました。", ex);
            MessageBox.Show(this, $"エクスポートに失敗しました。\n\n{ex.Message}",
                "エラー", MessageBoxButton.OK, MessageBoxImage.Error);
        }
        finally
        {
            Mouse.OverrideCursor = null;
        }
    }

    private void OnImport(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFileDialog
        {
            Title = "インポートするファイルを選択",
            Filter = "お気に入りショートカット データ (*.zip)|*.zip",
            CheckFileExists = true,
        };
        if (dialog.ShowDialog(this) != true) return;

        var manifest = _app.Transfer.PeekManifest(dialog.FileName);
        if (manifest is null)
        {
            MessageBox.Show(this,
                "このファイルは、このアプリのエクスポートデータではないようです。",
                "インポートできません", MessageBoxButton.OK, MessageBoxImage.Warning);
            return;
        }

        var modeDialog = new ImportModeWindow(manifest) { Owner = this };
        if (modeDialog.ShowDialog() != true) return;

        try
        {
            Mouse.OverrideCursor = Cursors.Wait;
            var summary = _app.Transfer.Import(dialog.FileName, modeDialog.SelectedMode);

            _icons.ClearMemoryCache();
            _scope = _allNode;
            BuildFolderTree();
            SelectScopeInTree();
            RefreshTagCloud();
            RefreshList();
            ApplySettings();

            var skipped = summary.SkippedShortcuts > 0
                ? $"\n重複のためスキップ: {summary.SkippedShortcuts} 件"
                : string.Empty;

            MessageBox.Show(this,
                $"インポートが完了しました。\n\n" +
                $"フォルダ: {summary.Folders} 件\nショートカット: {summary.Shortcuts} 件{skipped}\n\n" +
                $"念のため、取り込み前のデータを次の場所にバックアップしました:\n{summary.BackupPath}",
                "インポート完了", MessageBoxButton.OK, MessageBoxImage.Information);
        }
        catch (Exception ex)
        {
            AppLog.Error("インポートに失敗しました。", ex);
            MessageBox.Show(this,
                $"インポートに失敗しました。現在のデータは変更されていないか、" +
                $"バックアップから復元できます。\n\n{ex.Message}",
                "エラー", MessageBoxButton.OK, MessageBoxImage.Error);
        }
        finally
        {
            Mouse.OverrideCursor = null;
        }
    }

    private void OnBackupNow(object sender, RoutedEventArgs e)
    {
        try
        {
            var path = _app.Transfer.CreateBackup("manual");
            MessageBox.Show(this, $"バックアップを作成しました。\n\n{path}",
                "バックアップ", MessageBoxButton.OK, MessageBoxImage.Information);
        }
        catch (Exception ex)
        {
            AppLog.Error("バックアップに失敗しました。", ex);
            MessageBox.Show(this, $"バックアップに失敗しました。\n\n{ex.Message}",
                "エラー", MessageBoxButton.OK, MessageBoxImage.Error);
        }
    }

    private void OnOpenDataFolder(object sender, RoutedEventArgs e) =>
        LaunchService.OpenPath(AppPaths.DataDirectory);

    private void OnShowHelp(object sender, RoutedEventArgs e)
    {
        var readme = Path.Combine(AppPaths.ExeDirectory, "README.md");
        if (File.Exists(readme))
        {
            LaunchService.OpenPath(readme);
            return;
        }

        MessageBox.Show(this,
            "基本的な使い方\n\n" +
            "・「＋ 新規登録」または エクスプローラーからのドラッグ＆ドロップで登録します。\n" +
            "・タグは入力して Space キーで確定します（日本語IMEの変換用スペースとは競合しません）。\n" +
            "・上部の検索欄では、タイトル・タグ・フォルダ名・URL / パスをまとめて検索できます。\n" +
            "・どのアプリを使っていても、ホットキー（既定 Ctrl+Space）でランチャーを呼び出せます。\n" +
            "・ランチャーでは ↑↓ で選択、Enter で起動、Esc で閉じます。",
            "使い方", MessageBoxButton.OK, MessageBoxImage.Information);
    }

    // ------------------------------------------------------------- ユーティリティ

    private static T? FindAncestor<T>(DependencyObject? current) where T : DependencyObject
    {
        while (current is not null)
        {
            if (current is T typed) return typed;
            current = current is Visual or System.Windows.Media.Media3D.Visual3D
                ? VisualTreeHelper.GetParent(current)
                : LogicalTreeHelper.GetParent(current);
        }
        return null;
    }
}

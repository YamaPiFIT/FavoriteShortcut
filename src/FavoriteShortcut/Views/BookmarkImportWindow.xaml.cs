using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;

namespace FavoriteShortcut.Views;

/// <summary>
/// Edge / Chrome などのお気に入りを取り込むダイアログ。
///
/// ブラウザのファイルは読み取るだけで、一切書き込まない。
/// 取り込み先・フォルダ構成の扱い・重複の扱いを選んでから実行する。
/// </summary>
public partial class BookmarkImportWindow : Window
{
    private sealed record FolderChoice(string? Id, string Label)
    {
        public override string ToString() => Label;
    }

    private readonly AppStore _store;
    private readonly IconService _icons;

    private List<BookmarkNode> _roots = new();
    private readonly Dictionary<BookmarkNode, CheckBox> _rootChecks = new();

    public BookmarkImportWindow(AppStore store, IconService icons, string? currentFolderId)
    {
        InitializeComponent();

        _store = store;
        _icons = icons;

        BuildFolderChoices(currentFolderId);
        LoadProfiles();
    }

    /// <summary>取り込んだ件数（呼び出し元の通知用）。</summary>
    public int ImportedCount { get; private set; }

    public int SkippedCount { get; private set; }

    // --------------------------------------------------------- 初期化

    private void LoadProfiles()
    {
        List<BookmarkProfile> profiles;
        try
        {
            profiles = BrowserBookmarkReader.FindProfiles();
        }
        catch (Exception ex)
        {
            AppLog.Error("ブラウザのお気に入りを探せませんでした。", ex);
            profiles = new List<BookmarkProfile>();
        }

        ProfileCombo.ItemsSource = profiles;

        if (profiles.Count == 0)
        {
            ProfileCombo.IsEnabled = false;
            ImportButton.IsEnabled = false;
            StatusText.Text =
                "お気に入りを読み取れるブラウザが見つかりませんでした。" +
                "Microsoft Edge または Google Chrome がインストールされ、一度でも起動されている必要があります。";
            return;
        }

        ProfileCombo.SelectedIndex = 0;
    }

    private void BuildFolderChoices(string? selectedId)
    {
        var choices = new List<FolderChoice> { new(null, "未分類（フォルダに入れない）") };
        choices.AddRange(_store.AllFolders
            .OrderBy(f => f.FullPath, StringComparer.CurrentCulture)
            .Select(f => new FolderChoice(f.Id, f.FullPath)));

        FolderCombo.ItemsSource = choices;
        FolderCombo.SelectedItem = choices.FirstOrDefault(c => c.Id == selectedId) ?? choices[0];
    }

    private void OnProfileChanged(object sender, SelectionChangedEventArgs e)
    {
        if (ProfileCombo.SelectedItem is not BookmarkProfile profile) return;

        RootList.Children.Clear();
        _rootChecks.Clear();

        try
        {
            _roots = BrowserBookmarkReader.ReadRoots(profile.FilePath);
        }
        catch (Exception ex)
        {
            AppLog.Error($"お気に入りを読めませんでした: {profile.FilePath}", ex);
            _roots = new List<BookmarkNode>();
            StatusText.Text = $"このブラウザのお気に入りを読めませんでした。\n{ex.Message}";
        }

        // 既定のフォルダ名を、選んだブラウザに合わせる
        NewFolderNameBox.Text = $"{profile.BrowserName} のお気に入り";
        TagNameBox.Text = profile.BrowserName;

        foreach (var root in _roots)
        {
            var check = new CheckBox
            {
                IsChecked = root.LinkCount > 0,
                IsEnabled = root.LinkCount > 0,
                Margin = new Thickness(4, 5, 4, 5),
                Content = BuildRootLabel(root),
            };
            check.Checked += OnOptionChanged;
            check.Unchecked += OnOptionChanged;

            _rootChecks[root] = check;
            RootList.Children.Add(check);
        }

        if (_roots.Count == 0)
        {
            RootList.Children.Add(new TextBlock
            {
                Text = "お気に入りが登録されていません。",
                Margin = new Thickness(6),
                Style = (Style)FindResource("CaptionText"),
            });
        }

        UpdateSummary();
    }

    private static object BuildRootLabel(BookmarkNode root)
    {
        var panel = new StackPanel { Orientation = Orientation.Horizontal };
        panel.Children.Add(new TextBlock { Text = "📁", Margin = new Thickness(0, 0, 7, 0) });
        panel.Children.Add(new TextBlock
        {
            Text = root.Name.Length > 0 ? root.Name : "（名前なし）",
            FontWeight = FontWeights.SemiBold,
        });
        panel.Children.Add(new TextBlock
        {
            Text = $"　{root.LinkCount} 件",
            Opacity = 0.7,
        });
        return panel;
    }

    private void OnDestinationChanged(object sender, RoutedEventArgs e)
    {
        // XAML の初期化中にも Checked が飛んでくるので、生成済みか確認する
        if (NewFolderNameBox is null || FolderCombo is null) return;

        NewFolderNameBox.IsEnabled = NewFolderOption.IsChecked == true;
        FolderCombo.IsEnabled = ExistingFolderOption.IsChecked == true;
    }

    private void OnOptionChanged(object sender, RoutedEventArgs e)
    {
        if (TagNameBox is not null && AddTagCheck is not null)
            TagNameBox.IsEnabled = AddTagCheck.IsChecked == true;

        UpdateSummary();
    }

    private IEnumerable<BookmarkNode> SelectedRoots =>
        _roots.Where(r => _rootChecks.TryGetValue(r, out var c) && c.IsChecked == true);

    private void UpdateSummary()
    {
        if (SummaryText is null) return;

        var links = SelectedRoots.Sum(r => r.LinkCount);
        var folders = KeepStructureCheck?.IsChecked == true
            ? SelectedRoots.Sum(r => 1 + r.FolderCount)
            : 0;

        SummaryText.Text = folders > 0
            ? $"ショートカット {links} 件 / フォルダ {folders} 個"
            : $"ショートカット {links} 件";

        if (ImportButton is not null) ImportButton.IsEnabled = links > 0;
    }

    // --------------------------------------------------------- 取り込み

    private void OnImport(object sender, RoutedEventArgs e)
    {
        var selected = SelectedRoots.ToList();
        if (selected.Count == 0) return;

        string? destinationId;
        try
        {
            destinationId = ResolveDestinationFolder();
        }
        catch (Exception ex)
        {
            StatusText.Text = ex.Message;
            return;
        }

        var keepStructure = KeepStructureCheck.IsChecked == true;
        var skipDuplicates = SkipDuplicatesCheck.IsChecked == true;
        var tag = AddTagCheck.IsChecked == true ? TagNameBox.Text.Trim() : string.Empty;

        // 既存の URL は大文字小文字を無視して比較する
        var existing = skipDuplicates
            ? _store.Shortcuts.Select(s => s.Target).ToHashSet(StringComparer.OrdinalIgnoreCase)
            : new HashSet<string>(StringComparer.OrdinalIgnoreCase);

        Mouse.OverrideCursor = Cursors.Wait;
        try
        {
            foreach (var root in selected)
            {
                // 「ブックマーク バー」などの最上位フォルダも、構成を再現するなら作る
                var rootFolderId = keepStructure
                    ? _store.CreateFolder(SafeFolderName(root.Name), destinationId).Id
                    : destinationId;

                ImportChildren(root, rootFolderId, keepStructure, skipDuplicates, existing, tag);
            }
        }
        catch (Exception ex)
        {
            AppLog.Error("お気に入りの取り込みに失敗しました。", ex);
            MessageBox.Show(this,
                $"取り込み中にエラーが発生しました。\n\nそこまでに取り込んだ内容は保存されています。\n\n{ex.Message}",
                "エラー", MessageBoxButton.OK, MessageBoxImage.Error);
        }
        finally
        {
            Mouse.OverrideCursor = null;
        }

        AppLog.Info($"お気に入りを取り込みました: {ImportedCount} 件（スキップ {SkippedCount} 件）");
        DialogResult = true;
    }

    private void ImportChildren(
        BookmarkNode folder, string? folderId, bool keepStructure, bool skipDuplicates,
        HashSet<string> existing, string tag)
    {
        foreach (var child in folder.Children)
        {
            if (child.IsFolder)
            {
                var childFolderId = keepStructure
                    ? _store.CreateFolder(SafeFolderName(child.Name), folderId).Id
                    : folderId;

                ImportChildren(child, childFolderId, keepStructure, skipDuplicates, existing, tag);
                continue;
            }

            var url = child.Url!;
            if (skipDuplicates && !existing.Add(url))
            {
                SkippedCount++;
                continue;
            }

            var type = TargetResolver.Detect(url);
            var item = new ShortcutItem
            {
                Title = child.Name,
                Target = TargetResolver.Normalize(url, type),
                TargetType = type,
                FolderId = folderId,
                Tags = tag.Length > 0 ? new List<string> { tag } : new List<string>(),
            };

            _store.AddShortcut(item);

            // ブラウザから取り込んだ項目は、そのブラウザがアイコンを持っている可能性が高い
            _ = _icons.EnsureFaviconAsync(item, force: false);

            ImportedCount++;
        }
    }

    /// <summary>取り込み先フォルダを決める。新規作成を選んでいればここで作る。</summary>
    private string? ResolveDestinationFolder()
    {
        if (ExistingFolderOption.IsChecked == true)
            return (FolderCombo.SelectedItem as FolderChoice)?.Id;

        var name = NewFolderNameBox.Text.Trim();
        if (name.Length == 0)
            throw new InvalidOperationException("作成するフォルダ名を入力してください。");

        return _store.CreateFolder(name, null).Id;
    }

    /// <summary>ブラウザ側で名前のないフォルダがあっても破綻しないようにする。</summary>
    private static string SafeFolderName(string name)
    {
        var trimmed = name.Trim();
        return trimmed.Length == 0 ? "（名前なし）" : trimmed;
    }
}

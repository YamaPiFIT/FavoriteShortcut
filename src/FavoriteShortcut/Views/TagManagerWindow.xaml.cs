using System.Windows;
using System.Windows.Controls;
using FavoriteShortcut.Data;
using FavoriteShortcut.Services;

namespace FavoriteShortcut.Views;

/// <summary>タグの一覧・名前変更・削除。名前変更は使用中の全ショートカットに反映される。</summary>
public partial class TagManagerWindow : Window
{
    public sealed record TagRow(string Name, int Count)
    {
        public string CountText => Loc.T("Str.Common.CountItems", Count);
    }

    private readonly AppStore _store;
    private List<TagRow> _all = new();

    public TagManagerWindow(AppStore store)
    {
        InitializeComponent();
        _store = store;
        Reload();
    }

    private void Reload()
    {
        _all = _store.GetTagUsage().Select(t => new TagRow(t.Name, t.Count)).ToList();
        ApplyFilter();
    }

    private void ApplyFilter()
    {
        var filter = FilterBox.Text.Trim();
        FilterPlaceholder.Visibility = filter.Length == 0 ? Visibility.Visible : Visibility.Collapsed;

        TagList.ItemsSource = filter.Length == 0
            ? _all
            : _all.Where(t => t.Name.Contains(filter, StringComparison.OrdinalIgnoreCase)).ToList();
    }

    private void OnFilterChanged(object sender, TextChangedEventArgs e) => ApplyFilter();

    private void OnRename(object sender, RoutedEventArgs e)
    {
        if (TagList.SelectedItem is not TagRow row) return;

        var dialog = new TextInputWindow(Loc.T("Str.TagManager.RenameTitle"), Loc.T("Str.TagManager.RenamePrompt", row.Name), row.Name) { Owner = this };
        if (dialog.ShowDialog() != true) return;

        _store.RenameTag(row.Name, dialog.Value);
        Reload();
    }

    private void OnDelete(object sender, RoutedEventArgs e)
    {
        if (TagList.SelectedItem is not TagRow row) return;

        var answer = MessageBox.Show(this,
            Loc.T("Str.TagManager.DeleteConfirm", row.Name, row.Count), Loc.T("Str.TagManager.DeleteTitle"), MessageBoxButton.OKCancel, MessageBoxImage.Warning);
        if (answer != MessageBoxResult.OK) return;

        _store.DeleteTag(row.Name);
        Reload();
    }
}

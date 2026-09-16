using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Input;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;

namespace FavoriteShortcut.Views;

/// <summary>ショートカットの移動先フォルダを選ぶダイアログ。</summary>
public partial class FolderPickerWindow : Window
{
    private readonly SpecialFolderNode _uncategorized =
        new(SpecialFolderKind.Uncategorized, "未分類（フォルダに入れない）", "◇");

    public FolderPickerWindow(AppStore store, string? currentFolderId)
    {
        InitializeComponent();

        Tree.ItemsSource = new CompositeCollection
        {
            _uncategorized,
            new CollectionContainer { Collection = store.RootFolders },
        };

        SelectedFolderId = currentFolderId;

        Loaded += (_, _) =>
        {
            ExpandAll(Tree);
            Tree.Focus();
        };
    }

    public string? SelectedFolderId { get; private set; }

    private void OnSelectionChanged(object sender, RoutedPropertyChangedEventArgs<object> e) =>
        SelectedFolderId = e.NewValue is FolderItem folder ? folder.Id : null;

    private void OnDoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (Tree.SelectedItem is null) return;
        DialogResult = true;
    }

    private void OnOk(object sender, RoutedEventArgs e) => DialogResult = true;

    private static void ExpandAll(ItemsControl parent)
    {
        parent.UpdateLayout();
        foreach (var item in parent.Items)
        {
            if (parent.ItemContainerGenerator.ContainerFromItem(item) is not TreeViewItem container) continue;
            container.IsExpanded = true;
            container.UpdateLayout();
            ExpandAll(container);
        }
    }
}

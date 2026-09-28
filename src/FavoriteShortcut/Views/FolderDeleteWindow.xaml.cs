using System.Windows;
using FavoriteShortcut.Services;

namespace FavoriteShortcut.Views;

/// <summary>
/// フォルダ削除の確認（§37）。
/// 中身の扱いを明示的に選ばせ、うっかりショートカットを失わないようにする。
/// </summary>
public partial class FolderDeleteWindow : Window
{
    public FolderDeleteWindow(string folderName, int subFolderCount, int shortcutCount)
    {
        InitializeComponent();

        MessageText.Text = Loc.T("Str.Common.ConfirmDeleteOne", folderName);

        var parts = new List<string>();
        if (subFolderCount > 0) parts.Add(Loc.T("Str.FolderDelete.SubFolders", subFolderCount));
        if (shortcutCount > 0) parts.Add(Loc.T("Str.FolderDelete.Shortcuts", shortcutCount));

        DetailText.Text = parts.Count == 0
            ? Loc.T("Str.FolderDelete.Empty")
            : Loc.T("Str.FolderDelete.Contains", string.Join(Loc.T("Str.Common.ListSeparator"), parts));

        // 中身が無ければ選択肢を出す意味がない
        OptionBox.Visibility = shortcutCount > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    public bool DeleteShortcuts => DeleteOption.IsChecked == true;

    private void OnDelete(object sender, RoutedEventArgs e) => DialogResult = true;
}

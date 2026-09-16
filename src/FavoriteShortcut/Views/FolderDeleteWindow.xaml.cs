using System.Windows;

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

        MessageText.Text = $"「{folderName}」を削除しますか？";

        var parts = new List<string>();
        if (subFolderCount > 0) parts.Add($"サブフォルダ {subFolderCount} 個");
        if (shortcutCount > 0) parts.Add($"ショートカット {shortcutCount} 件");

        DetailText.Text = parts.Count == 0
            ? "このフォルダは空です。この操作は元に戻せません。"
            : $"このフォルダには {string.Join("、", parts)} が含まれています。" +
              "サブフォルダは一緒に削除されます。この操作は元に戻せません。";

        // 中身が無ければ選択肢を出す意味がない
        OptionBox.Visibility = shortcutCount > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    public bool DeleteShortcuts => DeleteOption.IsChecked == true;

    private void OnDelete(object sender, RoutedEventArgs e) => DialogResult = true;
}

using System.Globalization;
using System.Windows;
using FavoriteShortcut.Services;

namespace FavoriteShortcut.Views;

/// <summary>インポート方法（追加 / 置き換え）を選ぶダイアログ（§27）。</summary>
public partial class ImportModeWindow : Window
{
    public ImportModeWindow(ExportManifest manifest)
    {
        InitializeComponent();

        SummaryText.Text =
            $"フォルダ {manifest.FolderCount} 件 / ショートカット {manifest.ShortcutCount} 件 / タグ {manifest.TagCount} 件";

        var exported = DateTime.TryParse(manifest.ExportedAt, CultureInfo.InvariantCulture,
            DateTimeStyles.RoundtripKind, out var dt)
            ? dt.ToLocalTime().ToString("yyyy/MM/dd HH:mm")
            : manifest.ExportedAt;

        ExportedAtText.Text = $"エクスポート日時: {exported}　/　作成バージョン: {manifest.AppVersion}";
    }

    public ImportMode SelectedMode => ReplaceOption.IsChecked == true ? ImportMode.Replace : ImportMode.Merge;

    private void OnImport(object sender, RoutedEventArgs e)
    {
        if (SelectedMode == ImportMode.Replace)
        {
            var answer = MessageBox.Show(this,
                "現在のフォルダ・ショートカット・タグ・設定をすべて削除して、取り込んだ内容に置き換えます。\n\n" +
                "よろしいですか？",
                "置き換えの確認", MessageBoxButton.OKCancel, MessageBoxImage.Warning);
            if (answer != MessageBoxResult.OK) return;
        }

        DialogResult = true;
    }
}

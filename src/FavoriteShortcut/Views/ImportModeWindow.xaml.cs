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
            Loc.T("Str.ImportMode.Summary", manifest.FolderCount, manifest.ShortcutCount, manifest.TagCount);

        var exported = DateTime.TryParse(manifest.ExportedAt, CultureInfo.InvariantCulture,
            DateTimeStyles.RoundtripKind, out var dt)
            ? dt.ToLocalTime().ToString("yyyy/MM/dd HH:mm")
            : manifest.ExportedAt;

        ExportedAtText.Text = Loc.T("Str.ImportMode.ExportedAt", exported, manifest.AppVersion);
    }

    public ImportMode SelectedMode => ReplaceOption.IsChecked == true ? ImportMode.Replace : ImportMode.Merge;

    private void OnImport(object sender, RoutedEventArgs e)
    {
        if (SelectedMode == ImportMode.Replace)
        {
            var answer = MessageBox.Show(this,
                Loc.T("Str.ImportMode.ReplaceConfirm"), Loc.T("Str.ImportMode.ReplaceConfirmTitle"), MessageBoxButton.OKCancel, MessageBoxImage.Warning);
            if (answer != MessageBoxResult.OK) return;
        }

        DialogResult = true;
    }
}

using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using FavoriteShortcut.Data;
using FavoriteShortcut.Models;
using FavoriteShortcut.Services;
using Microsoft.Win32;

namespace FavoriteShortcut.Views;

/// <summary>ショートカットの新規登録・編集（§8）。</summary>
public partial class ShortcutEditWindow : Window
{
    private sealed record FolderChoice(string? Id, string Label)
    {
        public override string ToString() => Label;
    }

    private readonly App _app = App.Instance;
    private readonly AppStore _store;
    private readonly IconService _icons;

    private readonly ShortcutItem? _original;
    private readonly DispatcherTimer _targetTimer;

    /// <summary>ダイアログ上で選択中のアイコン（icons/ 配下の相対ファイル名）。null なら自動。</summary>
    private string? _iconPath;

    /// <summary>この画面で新しく取り込んだカスタムアイコン。キャンセル時に後始末する。</summary>
    private readonly List<string> _temporaryIcons = new();

    private TargetType _detectedType = TargetType.Unknown;

    /// <param name="existing">編集する既存項目。新規登録なら null。</param>
    /// <param name="defaultFolderId">新規登録時の初期フォルダ。</param>
    /// <param name="initialTarget">ドラッグ＆ドロップなどで最初から入れておく URL / パス。</param>
    public ShortcutEditWindow(ShortcutItem? existing, string? defaultFolderId = null, string? initialTarget = null)
    {
        InitializeComponent();

        _store = _app.Store;
        _icons = _app.Icons;
        _original = existing;

        _targetTimer = new DispatcherTimer(DispatcherPriority.Background)
        {
            Interval = TimeSpan.FromMilliseconds(250),
        };
        _targetTimer.Tick += (_, _) =>
        {
            _targetTimer.Stop();
            UpdateTargetInfo();
        };

        BuildFolderChoices(existing?.FolderId ?? defaultFolderId);
        TagInput.SetAvailableTags(_store.AllTagNames);

        if (existing is not null)
        {
            Title = Loc.T("Str.Edit.TitleEdit");
            SaveButton.Content = Loc.T("Str.Common.Save");
            DeleteButton.Visibility = Visibility.Visible;

            TargetBox.Text = existing.Target;
            TitleBox.Text = existing.Title;
            NoteBox.Text = existing.Note ?? string.Empty;
            TagInput.SetTags(existing.Tags);
            _iconPath = existing.IconPath;
        }
        else
        {
            Title = Loc.T("Str.Edit.TitleNew");
            SaveButton.Content = Loc.T("Str.Edit.Add");
            if (!string.IsNullOrWhiteSpace(initialTarget)) TargetBox.Text = initialTarget.Trim();
        }

        UpdateTargetInfo();

        Loaded += (_, _) =>
        {
            if (TargetBox.Text.Length == 0) TargetBox.Focus();
            else if (TitleBox.Text.Length == 0) TitleBox.Focus();
            else { TitleBox.Focus(); TitleBox.SelectAll(); }
        };
    }

    // ------------------------------------------------------------- フォルダ

    private void BuildFolderChoices(string? selectedId)
    {
        var choices = new List<FolderChoice> { new(null, Loc.T("Str.Folder.UncategorizedChoice")) };
        choices.AddRange(_store.AllFolders
            .OrderBy(f => f.FullPath, StringComparer.CurrentCulture)
            .Select(f => new FolderChoice(f.Id, f.FullPath)));

        FolderCombo.ItemsSource = choices;
        FolderCombo.SelectedItem = choices.FirstOrDefault(c => c.Id == selectedId) ?? choices[0];
    }

    private void OnNewFolder(object sender, RoutedEventArgs e)
    {
        var parentId = (FolderCombo.SelectedItem as FolderChoice)?.Id;
        var dialog = new TextInputWindow(Loc.T("Str.Folder.NewTitle"), Loc.T("Str.Folder.NameLabel"), string.Empty) { Owner = this };
        if (dialog.ShowDialog() != true) return;

        var created = _store.CreateFolder(dialog.Value, parentId);
        BuildFolderChoices(created.Id);
    }

    // ------------------------------------------------------ URL / パスの判定

    private void OnTargetChanged(object sender, TextChangedEventArgs e)
    {
        _targetTimer.Stop();
        _targetTimer.Start();
    }

    private void UpdateTargetInfo()
    {
        var raw = TargetBox.Text.Trim();
        _detectedType = TargetResolver.Detect(raw);

        if (raw.Length == 0)
        {
            TargetInfoText.Text = Loc.T("Str.Edit.TargetHint");
            FetchFaviconButton.IsEnabled = false;
            UpdateIconPreview();
            return;
        }

        var normalized = TargetResolver.Normalize(raw, _detectedType);
        var exists = TargetResolver.Exists(normalized, _detectedType);

        var info = Loc.T("Str.Edit.TypeInfo", _detectedType.ToDisplayName());
        if (_detectedType == TargetType.Web && !string.Equals(normalized, raw, StringComparison.Ordinal))
            info += Loc.T("Str.Edit.SavedAs", normalized);
        if (!exists)
            info += Loc.T("Str.Edit.PathMissing");

        TargetInfoText.Text = info;
        FetchFaviconButton.IsEnabled = _detectedType == TargetType.Web;

        // タイトル未入力なら、対象から推測した名前を入れておく
        if (TitleBox.Text.Trim().Length == 0)
            TitleBox.Text = TargetResolver.SuggestTitle(normalized, _detectedType);

        UpdateIconPreview();
    }

    // ------------------------------------------------------------- アイコン

    private void UpdateIconPreview()
    {
        var preview = new ShortcutItem
        {
            Target = TargetResolver.Normalize(TargetBox.Text.Trim(), _detectedType),
            TargetType = _detectedType,
            IconPath = _iconPath,
        };

        IconPreview.Source = _icons.ResolveSync(preview);

        IconSourceText.Text = _iconPath switch
        {
            null => _detectedType switch
            {
                TargetType.Web => Loc.T("Str.Edit.IconAutoWeb"),
                TargetType.Folder => Loc.T("Str.Edit.IconAutoFolder"),
                TargetType.File or TargetType.Application => Loc.T("Str.Edit.IconAutoFile"),
                _ => Loc.T("Str.Edit.IconAutoDefault"),
            },
            var p when p.StartsWith("custom_", StringComparison.Ordinal) => Loc.T("Str.Edit.IconCustom"),
            _ => Loc.T("Str.Edit.IconCached"),
        };
    }

    private void OnChooseIcon(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFileDialog
        {
            Title = Loc.T("Str.Edit.ChooseImageTitle"),
            Filter = Loc.T("Str.Edit.ImageFilter"),
            CheckFileExists = true,
        };
        if (dialog.ShowDialog(this) != true) return;

        var saved = _icons.ImportCustomIcon(dialog.FileName);
        if (saved is null)
        {
            MessageBox.Show(this, Loc.T("Str.Edit.ImageUnreadable"),
                Loc.T("Str.Edit.Icon"), MessageBoxButton.OK, MessageBoxImage.Warning);
            return;
        }

        ReplaceIcon(saved);
        _temporaryIcons.Add(saved);
    }

    private async void OnFetchFavicon(object sender, RoutedEventArgs e)
    {
        var target = TargetResolver.Normalize(TargetBox.Text.Trim(), TargetType.Web);
        if (TargetResolver.TryGetHost(target) is null)
        {
            MessageBox.Show(this, Loc.T("Str.Edit.NoHost"), Loc.T("Str.Edit.FetchFavicon"),
                MessageBoxButton.OK, MessageBoxImage.Warning);
            return;
        }

        var probe = new ShortcutItem { Target = target, TargetType = TargetType.Web };

        FetchFaviconButton.IsEnabled = false;
        FetchFaviconButton.Content = Loc.T("Str.Edit.Fetching");
        try
        {
            await _icons.EnsureFaviconAsync(probe, force: true);
        }
        finally
        {
            FetchFaviconButton.Content = Loc.T("Str.Edit.FetchFavicon");
            FetchFaviconButton.IsEnabled = true;
        }

        if (probe.IconPath is null)
        {
            MessageBox.Show(this,
                Loc.T("Str.Edit.FaviconFailed"), Loc.T("Str.Edit.FetchFavicon"), MessageBoxButton.OK, MessageBoxImage.Information);
            return;
        }

        ReplaceIcon(probe.IconPath);
    }

    private void OnResetIcon(object sender, RoutedEventArgs e) => ReplaceIcon(null);

    private void ReplaceIcon(string? newIconPath)
    {
        _iconPath = newIconPath;
        UpdateIconPreview();
    }

    // ---------------------------------------------------------------- 参照

    private void OnBrowseFile(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFileDialog
        {
            Title = Loc.T("Str.Edit.ChooseFileTitle"),
            Filter = Loc.T("Str.Edit.AllFilesFilter"),
            CheckFileExists = true,
        };
        if (dialog.ShowDialog(this) != true) return;

        TargetBox.Text = dialog.FileName;
        UpdateTargetInfo();
    }

    private void OnBrowseFolder(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog { Title = Loc.T("Str.Edit.ChooseFolderTitle") };
        if (dialog.ShowDialog(this) != true) return;

        TargetBox.Text = dialog.FolderName;
        UpdateTargetInfo();
    }

    // ---------------------------------------------------------------- 保存

    private void OnSave(object sender, RoutedEventArgs e)
    {
        // 入力途中のタグを取りこぼさない
        TagInput.CommitPending();

        var rawTarget = TargetBox.Text.Trim();
        if (rawTarget.Length == 0)
        {
            ShowError(Loc.T("Str.Edit.TargetRequired"));
            TargetBox.Focus();
            return;
        }

        _detectedType = TargetResolver.Detect(rawTarget);
        var target = TargetResolver.Normalize(rawTarget, _detectedType);

        var title = TitleBox.Text.Trim();
        if (title.Length == 0) title = TargetResolver.SuggestTitle(target, _detectedType);
        if (title.Length == 0)
        {
            ShowError(Loc.T("Str.Edit.TitleRequired"));
            TitleBox.Focus();
            return;
        }

        if (!ConfirmDuplicateIfNeeded(target)) return;

        var folderId = (FolderCombo.SelectedItem as FolderChoice)?.Id;
        var note = NoteBox.Text.Trim();

        try
        {
            if (_original is null)
            {
                var item = new ShortcutItem
                {
                    Title = title,
                    Target = target,
                    TargetType = _detectedType,
                    FolderId = folderId,
                    IconPath = _iconPath,
                    Note = note.Length == 0 ? null : note,
                    Tags = TagInput.Tags.ToList(),
                };
                _store.AddShortcut(item);
                if (item.TargetType == TargetType.Web && item.IconPath is null)
                    _ = _icons.EnsureFaviconAsync(item, force: false);
            }
            else
            {
                // アイコンを差し替えたなら、使われなくなったカスタム画像を片付ける
                if (_original.IconPath != _iconPath)
                    _icons.DeleteCustomIcon(_original.IconPath);

                _original.Title = title;
                _original.Target = target;
                _original.TargetType = _detectedType;
                _original.FolderId = folderId;
                _original.IconPath = _iconPath;
                _original.Note = note.Length == 0 ? null : note;
                _original.Tags = TagInput.Tags.ToList();

                _store.UpdateShortcut(_original);
                _icons.Attach(_original);
            }

            // 保存できたので、この画面で取り込んだ画像は後始末の対象から外す
            _temporaryIcons.Clear();
            DialogResult = true;
        }
        catch (Exception ex)
        {
            AppLog.Error("ショートカットの保存に失敗しました。", ex);
            ShowError(Loc.T("Str.Edit.SaveFailed", ex.Message));
        }
    }

    /// <summary>同じ URL / パスが既に登録されている場合に確認する（§38: 重複自体は許可する）。</summary>
    private bool ConfirmDuplicateIfNeeded(string target)
    {
        var duplicate = _store.Shortcuts.FirstOrDefault(s =>
            !ReferenceEquals(s, _original) &&
            string.Equals(s.Target, target, StringComparison.OrdinalIgnoreCase));

        if (duplicate is null) return true;

        var answer = MessageBox.Show(this,
            Loc.T("Str.Edit.Duplicate", duplicate.DisplayTitle, duplicate.FolderPathText), Loc.T("Str.Edit.DuplicateTitle"), MessageBoxButton.OKCancel, MessageBoxImage.Question);

        return answer == MessageBoxResult.OK;
    }

    private void OnDelete(object sender, RoutedEventArgs e)
    {
        if (_original is null) return;

        var answer = MessageBox.Show(this,
            Loc.T("Str.Common.ConfirmDeleteOne", _original.DisplayTitle) + "\n\n" + Loc.T("Str.Common.CannotUndo"),
            Loc.T("Str.Common.ConfirmDeleteTitle"), MessageBoxButton.OKCancel, MessageBoxImage.Warning);
        if (answer != MessageBoxResult.OK) return;

        _icons.DeleteCustomIcon(_original.IconPath);
        _store.DeleteShortcut(_original);
        DialogResult = true;
    }

    private void ShowError(string message)
    {
        ErrorText.Text = message;
        ErrorText.Visibility = Visibility.Visible;
    }

    protected override void OnClosed(EventArgs e)
    {
        // キャンセルで閉じた場合、この画面で取り込んだ画像ファイルは不要なので削除する
        if (DialogResult != true)
            foreach (var icon in _temporaryIcons) _icons.DeleteCustomIcon(icon);

        base.OnClosed(e);
    }
}

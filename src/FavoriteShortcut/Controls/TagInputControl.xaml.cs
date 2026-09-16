using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace FavoriteShortcut.Controls;

/// <summary>
/// YouTube 風のタグ入力（§5）。
///
/// 文字を入力して Space を押すとタグチップとして確定する。
///
/// 日本語IMEへの配慮（§32）:
///   - IME が変換中のキーは WPF では Key.ImeProcessed として通知される。
///     これを最初に弾くので、「かいはつ」→ Space で変換 の操作がタグ確定にならない。
///   - IME が全角スペースを直接入力した場合に備え、TextChanged 側でも
///     半角/全角スペースと読点・カンマを区切りとして扱う。
///   - 変換確定の Enter も Key.ImeProcessed になるため、タグ確定と衝突しない。
/// </summary>
public partial class TagInputControl : UserControl
{
    private readonly List<string> _tags = new();
    private IReadOnlyList<string> _available = Array.Empty<string>();

    /// <summary>候補リストをクリック中かどうか。LostFocus での二重確定を防ぐ。</summary>
    private bool _pickingSuggestion;

    /// <summary>タグの区切りとして扱う文字。全角スペース・読点も含める。</summary>
    private static readonly char[] Separators = { ' ', '　', ',', '、' };

    public TagInputControl()
    {
        InitializeComponent();
        UpdatePlaceholder();
    }

    /// <summary>タグが増減したときに発生する。</summary>
    public event EventHandler? TagsChanged;

    /// <summary>現在のタグ（表示順）。</summary>
    public IReadOnlyList<string> Tags => _tags;

    /// <summary>入力補完に使う既存タグの一覧。</summary>
    public void SetAvailableTags(IEnumerable<string> tags) =>
        _available = tags.Distinct(StringComparer.OrdinalIgnoreCase).ToList();

    public void SetTags(IEnumerable<string>? tags)
    {
        _tags.Clear();
        if (tags is not null)
        {
            foreach (var tag in tags)
            {
                var name = tag.Trim();
                if (name.Length == 0) continue;
                if (_tags.Any(t => string.Equals(t, name, StringComparison.OrdinalIgnoreCase))) continue;
                _tags.Add(name);
            }
        }
        RebuildChips();
    }

    /// <summary>入力欄に残っている文字もタグとして確定する（保存直前に呼ぶ）。</summary>
    public void CommitPending()
    {
        CommitCurrentText();
        HideSuggestions();
    }

    public new void Focus() => Input.Focus();

    // ------------------------------------------------------------- 入力処理

    private void OnInputPreviewKeyDown(object sender, KeyEventArgs e)
    {
        // IME が処理中のキー（変換のスペース・確定の Enter など）には一切手を出さない
        if (e.Key is Key.ImeProcessed or Key.DeadCharProcessed) return;

        switch (e.Key)
        {
            case Key.Space:
                if (CommitCurrentText()) e.Handled = true;
                // 空欄での Space は何もしない（誤って空タグを作らない）
                else e.Handled = Input.Text.Length == 0;
                return;

            case Key.Enter:
                if (SuggestionPopup.IsOpen && SuggestionList.SelectedItem is string picked)
                {
                    AddTag(picked);
                    Input.Clear();
                    HideSuggestions();
                    e.Handled = true;
                    return;
                }
                // 入力中の文字があるときだけタグ確定。空ならダイアログの既定ボタンへ流す。
                if (CommitCurrentText()) e.Handled = true;
                return;

            case Key.Tab:
                CommitCurrentText();
                HideSuggestions();
                return;

            case Key.Back:
                if (Input.Text.Length == 0 && Input.CaretIndex == 0 && _tags.Count > 0)
                {
                    RemoveTag(_tags[^1]);
                    e.Handled = true;
                }
                return;

            case Key.Down:
                if (SuggestionPopup.IsOpen && SuggestionList.Items.Count > 0)
                {
                    SuggestionList.SelectedIndex = Math.Min(SuggestionList.SelectedIndex + 1,
                        SuggestionList.Items.Count - 1);
                    SuggestionList.ScrollIntoView(SuggestionList.SelectedItem);
                    e.Handled = true;
                }
                return;

            case Key.Up:
                if (SuggestionPopup.IsOpen && SuggestionList.Items.Count > 0)
                {
                    SuggestionList.SelectedIndex = Math.Max(SuggestionList.SelectedIndex - 1, 0);
                    SuggestionList.ScrollIntoView(SuggestionList.SelectedItem);
                    e.Handled = true;
                }
                return;

            case Key.Escape:
                if (SuggestionPopup.IsOpen)
                {
                    HideSuggestions();
                    e.Handled = true;
                }
                return;
        }
    }

    private void OnInputTextChanged(object sender, TextChangedEventArgs e)
    {
        var text = Input.Text;

        // IME が全角スペースを直接入れた場合など、キー入力を経由しない区切りを拾う
        if (text.IndexOfAny(Separators) >= 0)
        {
            var parts = text.Split(Separators, StringSplitOptions.None);
            for (var i = 0; i < parts.Length - 1; i++) AddTag(parts[i]);

            var rest = parts[^1];
            Input.TextChanged -= OnInputTextChanged;
            Input.Text = rest;
            Input.CaretIndex = rest.Length;
            Input.TextChanged += OnInputTextChanged;
        }

        UpdatePlaceholder();
        UpdateSuggestions();
    }

    private void OnInputLostFocus(object sender, RoutedEventArgs e)
    {
        if (_pickingSuggestion) return;

        // フォーカスが外れたときに入力途中の文字を捨てず、タグとして確定する
        CommitCurrentText();
        HideSuggestions();
    }

    private void OnRootMouseDown(object sender, MouseButtonEventArgs e)
    {
        Input.Focus();
        Input.CaretIndex = Input.Text.Length;
    }

    private bool CommitCurrentText()
    {
        var text = Input.Text.Trim();
        if (text.Length == 0) return false;

        var added = false;
        foreach (var part in text.Split(Separators, StringSplitOptions.RemoveEmptyEntries))
            added |= AddTag(part);

        Input.Clear();
        UpdatePlaceholder();
        HideSuggestions();
        return added;
    }

    private bool AddTag(string name)
    {
        name = name.Trim();
        if (name.Length == 0) return false;
        if (name.Length > 60) name = name[..60];

        // 大文字小文字違いの同じタグは登録しない（§5）
        if (_tags.Any(t => string.Equals(t, name, StringComparison.OrdinalIgnoreCase))) return false;

        _tags.Add(name);
        RebuildChips();
        return true;
    }

    private void RemoveTag(string name)
    {
        _tags.RemoveAll(t => string.Equals(t, name, StringComparison.OrdinalIgnoreCase));
        RebuildChips();
    }

    // --------------------------------------------------------------- 見た目

    private void RebuildChips()
    {
        // 入力欄は残したまま、チップだけ作り直す
        for (var i = ChipPanel.Children.Count - 1; i >= 0; i--)
            if (!ReferenceEquals(ChipPanel.Children[i], Input))
                ChipPanel.Children.RemoveAt(i);

        var insertAt = 0;
        foreach (var tag in _tags)
            ChipPanel.Children.Insert(insertAt++, CreateChip(tag));

        UpdatePlaceholder();
        TagsChanged?.Invoke(this, EventArgs.Empty);
    }

    private UIElement CreateChip(string tag)
    {
        var text = new TextBlock
        {
            Text = tag,
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = (Brush)FindResource("TagForegroundBrush"),
        };

        var close = new Button
        {
            Content = "✕",
            FontSize = 10,
            Width = 18,
            Height = 18,
            Margin = new Thickness(6, 0, -2, 0),
            Padding = new Thickness(0),
            VerticalAlignment = VerticalAlignment.Center,
            Cursor = Cursors.Hand,
            ToolTip = $"「{tag}」を削除",
            Style = (Style)FindResource("GhostButton"),
        };
        close.Click += (_, _) => RemoveTag(tag);

        var content = new StackPanel { Orientation = Orientation.Horizontal };
        content.Children.Add(text);
        content.Children.Add(close);

        return new Border
        {
            Background = (Brush)FindResource("TagBackgroundBrush"),
            CornerRadius = new CornerRadius(11),
            Padding = new Thickness(10, 3, 6, 3),
            Margin = new Thickness(2),
            Child = content,
        };
    }

    private void UpdatePlaceholder() =>
        Placeholder.Visibility = _tags.Count == 0 && Input.Text.Length == 0
            ? Visibility.Visible
            : Visibility.Collapsed;

    // --------------------------------------------------------------- 補完

    private void UpdateSuggestions()
    {
        var text = Input.Text.Trim();
        if (text.Length == 0 || _available.Count == 0)
        {
            HideSuggestions();
            return;
        }

        var matches = _available
            .Where(t => !_tags.Any(x => string.Equals(x, t, StringComparison.OrdinalIgnoreCase)))
            .Where(t => t.Contains(text, StringComparison.OrdinalIgnoreCase))
            .OrderBy(t => t.StartsWith(text, StringComparison.OrdinalIgnoreCase) ? 0 : 1)
            .ThenBy(t => t.Length)
            .Take(8)
            .ToList();

        if (matches.Count == 0)
        {
            HideSuggestions();
            return;
        }

        SuggestionList.ItemsSource = matches;
        SuggestionList.SelectedIndex = -1;
        SuggestionPopup.IsOpen = true;
    }

    private void HideSuggestions()
    {
        SuggestionPopup.IsOpen = false;
        SuggestionList.ItemsSource = null;
    }

    private void OnSuggestionKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter && SuggestionList.SelectedItem is string tag)
        {
            AddTag(tag);
            Input.Clear();
            HideSuggestions();
            Input.Focus();
            e.Handled = true;
        }
        else if (e.Key == Key.Escape)
        {
            HideSuggestions();
            Input.Focus();
            e.Handled = true;
        }
    }

    /// <summary>
    /// 候補をクリックしたときの処理。
    /// MouseUp ではなく MouseDown で拾い、_pickingSuggestion を立ててから入力欄を空にする。
    /// こうしないと、フォーカス移動で走る LostFocus が
    /// 入力途中の文字まで別のタグとして確定してしまう。
    /// </summary>
    private void OnSuggestionClick(object sender, MouseButtonEventArgs e)
    {
        if (ItemsControl.ContainerFromElement(SuggestionList, (DependencyObject)e.OriginalSource)
                is not ListBoxItem { Content: string tag })
            return;

        _pickingSuggestion = true;
        try
        {
            AddTag(tag);
            Input.Clear();
            HideSuggestions();
            Input.Focus();
        }
        finally
        {
            _pickingSuggestion = false;
        }

        e.Handled = true;
    }
}

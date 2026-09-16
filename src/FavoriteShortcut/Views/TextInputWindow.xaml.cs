using System.Windows;
using System.Windows.Input;

namespace FavoriteShortcut.Views;

/// <summary>フォルダ名などを 1 行入力してもらう共通ダイアログ。</summary>
public partial class TextInputWindow : Window
{
    public TextInputWindow(string title, string label, string initialValue)
    {
        InitializeComponent();

        Title = title;
        LabelText.Text = label;
        ValueBox.Text = initialValue;

        Loaded += (_, _) =>
        {
            ValueBox.Focus();
            ValueBox.SelectAll();
        };
    }

    public string Value { get; private set; } = string.Empty;

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        // IME 変換確定の Enter でダイアログを閉じないようにする
        if (e.Key is Key.ImeProcessed or Key.DeadCharProcessed) return;

        if (e.Key == Key.Enter)
        {
            Commit();
            e.Handled = true;
        }
    }

    private void OnOk(object sender, RoutedEventArgs e) => Commit();

    private void Commit()
    {
        var value = ValueBox.Text.Trim();
        if (value.Length == 0)
        {
            ErrorText.Text = "1 文字以上入力してください。";
            ErrorText.Visibility = Visibility.Visible;
            ValueBox.Focus();
            return;
        }

        Value = value;
        DialogResult = true;
    }
}

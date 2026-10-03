using System.Windows;

namespace FavoriteShortcut.Controls;

/// <summary>
/// ドラッグ中に「ここに落とせる」ことを示すための添付プロパティ。
/// フォルダツリーで、ドロップ先のフォルダを強調表示するのに使う。
/// </summary>
public static class DropTarget
{
    public static readonly DependencyProperty IsActiveProperty = DependencyProperty.RegisterAttached(
        "IsActive", typeof(bool), typeof(DropTarget), new PropertyMetadata(false));

    public static bool GetIsActive(DependencyObject element) => (bool)element.GetValue(IsActiveProperty);

    public static void SetIsActive(DependencyObject element, bool value) => element.SetValue(IsActiveProperty, value);
}

using FavoriteShortcut.Models;
using Microsoft.Win32;

namespace FavoriteShortcut.Services;

/// <summary>
/// テーマの「自動」のために、Windows のアプリの配色（ライト / ダーク）を調べる。
/// 設定 → 個人用設定 → 色 →「既定のアプリ モードを選択します」に対応する。
/// </summary>
public static class SystemTheme
{
    private const string PersonalizeKey = @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

    /// <summary>このテーマ設定で、実際にダークで表示するか。</summary>
    public static bool IsDark(AppTheme theme) => theme switch
    {
        AppTheme.Dark => true,
        AppTheme.Auto => WindowsUsesDarkMode(),
        _ => false,
    };

    /// <summary>Windows のアプリの配色がダークか。読めない環境（古い Windows など）ではライトとみなす。</summary>
    public static bool WindowsUsesDarkMode()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(PersonalizeKey);
            return key?.GetValue("AppsUseLightTheme") is int value && value == 0;
        }
        catch (Exception ex)
        {
            AppLog.Warn("Windows の配色を読めませんでした。", ex);
            return false;
        }
    }
}

using Microsoft.Win32;

namespace FavoriteShortcut.Services;

/// <summary>
/// Windows ログオン時の自動起動（§66）。
/// 現在のユーザーの Run キーだけを操作するので、管理者権限は不要。
/// 設定画面のチェックでユーザーが明示的に選んだときだけ呼ばれる。
/// </summary>
public static class StartupService
{
    private const string RunKeyPath = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "FavoriteShortcut";

    /// <summary>自動起動時は --tray を付け、ウィンドウを出さずトレイ常駐で始める。</summary>
    public const string TrayArgument = "--tray";

    public static bool IsEnabled()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(RunKeyPath, writable: false);
            return key?.GetValue(ValueName) is string value && value.Length > 0;
        }
        catch (Exception ex)
        {
            AppLog.Warn("自動起動設定の読み取りに失敗しました。", ex);
            return false;
        }
    }

    /// <summary>設定を適用する。成功したら true。</summary>
    public static bool SetEnabled(bool enabled)
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(RunKeyPath, writable: true)
                            ?? Registry.CurrentUser.CreateSubKey(RunKeyPath, writable: true);
            if (key is null) return false;

            if (enabled)
            {
                var exe = Environment.ProcessPath;
                if (string.IsNullOrEmpty(exe)) return false;
                key.SetValue(ValueName, $"\"{exe}\" {TrayArgument}", RegistryValueKind.String);
            }
            else if (key.GetValue(ValueName) is not null)
            {
                key.DeleteValue(ValueName, throwOnMissingValue: false);
            }

            return true;
        }
        catch (Exception ex)
        {
            AppLog.Warn("自動起動設定の変更に失敗しました。", ex);
            return false;
        }
    }
}

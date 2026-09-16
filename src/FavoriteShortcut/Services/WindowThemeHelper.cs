using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace FavoriteShortcut.Services;

/// <summary>
/// Windows 10/11 のタイトルバーをテーマに合わせて暗くする。
/// 対応していない OS では何も起きないので、そのまま呼んで問題ない。
/// </summary>
public static class WindowThemeHelper
{
    // Windows 11 / Windows 10 20H1 以降
    private const int DWMWA_USE_IMMERSIVE_DARK_MODE = 20;
    // Windows 10 1809〜1903 の旧属性値
    private const int DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY = 19;

    public static void Apply(Window window, bool dark)
    {
        try
        {
            var handle = new WindowInteropHelper(window).Handle;
            if (handle == IntPtr.Zero) return;

            var value = dark ? 1 : 0;
            if (DwmSetWindowAttribute(handle, DWMWA_USE_IMMERSIVE_DARK_MODE, ref value, sizeof(int)) != 0)
                DwmSetWindowAttribute(handle, DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY, ref value, sizeof(int));
        }
        catch (Exception ex)
        {
            AppLog.Warn("タイトルバーのテーマ適用に失敗しました。", ex);
        }
    }

    /// <summary>開いているすべてのウィンドウに適用する（テーマ切り替え時）。</summary>
    public static void ApplyToAll(bool dark)
    {
        foreach (Window window in Application.Current.Windows)
            Apply(window, dark);
    }

    [DllImport("dwmapi.dll", PreserveSig = true)]
    private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int attrValue, int attrSize);
}

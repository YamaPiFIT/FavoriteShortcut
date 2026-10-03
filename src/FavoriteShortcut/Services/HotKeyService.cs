using System.Runtime.InteropServices;
using System.Windows.Input;
using System.Windows.Interop;

namespace FavoriteShortcut.Services;

/// <summary>
/// グローバルホットキー（§51, §63）。
///
/// 他のアプリを操作中でも反応する必要があるため、メッセージ専用ウィンドウを 1 つ作り、
/// Win32 の RegisterHotKey / WM_HOTKEY で受け取る。
/// ランチャーの呼び出しと「クリップボードから登録」の 2 つを扱う。
/// 同じウィンドウで「二重起動されたときに既存のウィンドウを前面に出す」メッセージも受ける。
/// </summary>
public sealed class HotKeyService : IDisposable
{
    private const int WM_HOTKEY = 0x0312;
    private const int LauncherHotKeyId = 0xA731;
    private const int ClipboardHotKeyId = 0xA732;

    private const int HWND_BROADCAST = 0xFFFF;
    private const int WS_POPUP = unchecked((int)0x80000000);
    private const int WS_EX_TOOLWINDOW = 0x00000080;

    private const uint MOD_ALT = 0x0001;
    private const uint MOD_CONTROL = 0x0002;
    private const uint MOD_SHIFT = 0x0004;
    private const uint MOD_WIN = 0x0008;
    private const uint MOD_NOREPEAT = 0x4000;

    /// <summary>二重起動時に、既存インスタンスへ「ウィンドウを出して」と伝えるためのメッセージ。</summary>
    public static readonly uint ShowWindowMessage = RegisterWindowMessage("FavoriteShortcut_ShowMainWindow");

    private HwndSource? _source;
    private readonly HashSet<int> _registered = new();

    /// <summary>ランチャーのホットキーが押された。</summary>
    public event EventHandler? HotKeyPressed;

    /// <summary>「クリップボードから登録」のホットキーが押された。</summary>
    public event EventHandler? ClipboardHotKeyPressed;

    public event EventHandler? ShowRequested;

    /// <summary>現在登録されているランチャーのホットキー。未登録なら null。</summary>
    public (ModifierKeys Modifiers, Key Key)? Current { get; private set; }

    /// <summary>現在登録されている「クリップボードから登録」のホットキー。未登録なら null。</summary>
    public (ModifierKeys Modifiers, Key Key)? ClipboardCurrent { get; private set; }

    public void Initialize()
    {
        if (_source is not null) return;

        // メッセージ専用ウィンドウ（HWND_MESSAGE）はブロードキャストメッセージを受け取れないため、
        // 二重起動の通知を受けられるよう、非表示のトップレベルウィンドウとして作る。
        // WS_EX_TOOLWINDOW を付けているので Alt+Tab やタスクバーには出ない。
        var parameters = new HwndSourceParameters("FavoriteShortcutHotKeyWindow")
        {
            WindowStyle = WS_POPUP,
            ExtendedWindowStyle = WS_EX_TOOLWINDOW,
            Width = 0,
            Height = 0,
            PositionX = -32000,
            PositionY = -32000,
        };
        _source = new HwndSource(parameters);
        _source.AddHook(WndProc);
    }

    /// <summary>
    /// ランチャーのホットキーを登録し直す。成功したら true。
    /// 他アプリが同じキーを使っている場合は false（設定画面で別のキーを選んでもらう）。
    /// </summary>
    public bool Register(ModifierKeys modifiers, Key key)
    {
        var ok = RegisterCore(LauncherHotKeyId, modifiers, key, "グローバルホットキー");
        Current = ok ? (modifiers, key) : null;
        return ok;
    }

    /// <summary>
    /// 「クリップボードから登録」のホットキーを登録し直す。成功したら true。
    /// キーが未設定（既定）の場合は解除だけして true を返す。
    /// </summary>
    public bool RegisterClipboard(ModifierKeys modifiers, Key key)
    {
        if (key == Key.None)
        {
            UnregisterCore(ClipboardHotKeyId);
            ClipboardCurrent = null;
            return true;
        }

        var ok = RegisterCore(ClipboardHotKeyId, modifiers, key, "「クリップボードから登録」のホットキー");
        ClipboardCurrent = ok ? (modifiers, key) : null;
        return ok;
    }

    private bool RegisterCore(int id, ModifierKeys modifiers, Key key, string purpose)
    {
        Initialize();
        UnregisterCore(id);

        if (key == Key.None) return false;

        var mods = ToWin32Modifiers(modifiers) | MOD_NOREPEAT;
        var vk = (uint)KeyInterop.VirtualKeyFromKey(key);
        if (vk == 0) return false;

        if (RegisterHotKey(_source!.Handle, id, mods, vk))
        {
            _registered.Add(id);
            AppLog.Info($"{purpose}を登録しました: {Describe(modifiers, key)}");
            return true;
        }

        AppLog.Warn($"{purpose}を登録できませんでした: {Describe(modifiers, key)}");
        return false;
    }

    /// <summary>ランチャーのホットキーを解除する。</summary>
    public void Unregister()
    {
        UnregisterCore(LauncherHotKeyId);
        Current = null;
    }

    private void UnregisterCore(int id)
    {
        if (_source is null || !_registered.Remove(id)) return;
        UnregisterHotKey(_source.Handle, id);
    }

    private IntPtr WndProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == WM_HOTKEY && wParam.ToInt32() == LauncherHotKeyId)
        {
            handled = true;
            HotKeyPressed?.Invoke(this, EventArgs.Empty);
        }
        else if (msg == WM_HOTKEY && wParam.ToInt32() == ClipboardHotKeyId)
        {
            handled = true;
            ClipboardHotKeyPressed?.Invoke(this, EventArgs.Empty);
        }
        else if (ShowWindowMessage != 0 && msg == ShowWindowMessage)
        {
            handled = true;
            ShowRequested?.Invoke(this, EventArgs.Empty);
        }

        return IntPtr.Zero;
    }

    /// <summary>すでに起動しているインスタンスへウィンドウ表示を依頼する。</summary>
    public static void BroadcastShowRequest()
    {
        if (ShowWindowMessage == 0) return;
        PostMessage(new IntPtr(HWND_BROADCAST), ShowWindowMessage, IntPtr.Zero, IntPtr.Zero);
    }

    public static string Describe(ModifierKeys modifiers, Key key)
    {
        if (key == Key.None) return Loc.T("Str.HotKey.NotSet");

        var parts = new List<string>();
        if (modifiers.HasFlag(ModifierKeys.Control)) parts.Add("Ctrl");
        if (modifiers.HasFlag(ModifierKeys.Shift)) parts.Add("Shift");
        if (modifiers.HasFlag(ModifierKeys.Alt)) parts.Add("Alt");
        if (modifiers.HasFlag(ModifierKeys.Windows)) parts.Add("Win");
        parts.Add(key.ToString());
        return string.Join(" + ", parts);
    }

    private static uint ToWin32Modifiers(ModifierKeys modifiers)
    {
        uint result = 0;
        if (modifiers.HasFlag(ModifierKeys.Alt)) result |= MOD_ALT;
        if (modifiers.HasFlag(ModifierKeys.Control)) result |= MOD_CONTROL;
        if (modifiers.HasFlag(ModifierKeys.Shift)) result |= MOD_SHIFT;
        if (modifiers.HasFlag(ModifierKeys.Windows)) result |= MOD_WIN;
        return result;
    }

    public void Dispose()
    {
        Unregister();
        UnregisterCore(ClipboardHotKeyId);
        ClipboardCurrent = null;
        _source?.RemoveHook(WndProc);
        _source?.Dispose();
        _source = null;
    }

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool RegisterHotKey(IntPtr hWnd, int id, uint fsModifiers, uint vk);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool UnregisterHotKey(IntPtr hWnd, int id);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern uint RegisterWindowMessage(string lpString);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
}

//! Windows との連携（二重起動の防止・グローバルホットキー・メッセージボックス・
//! クリップボード・自動起動・ウィンドウの小物）。C# 版と同じ仕組み・同じ名前を使う。

use std::sync::Mutex;

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey, VK_LWIN, VK_RWIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HWND_BROADCAST, IDOK, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK,
    MB_OKCANCEL, MESSAGEBOX_STYLE, MessageBoxW, PostMessageW, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, WM_HOTKEY,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::keys;
use crate::util::wide;

/// C# 版と同じ名前。同じデータを使う 2 つのアプリが同時に動かないようにする。
const MUTEX_NAME: &str = "Local\\FavoriteShortcut.SingleInstance";
/// 二重起動時に、既存インスタンスへ「ウィンドウを出して」と伝えるためのメッセージ（C# 版と同じ名前）。
const SHOW_MESSAGE_NAME: &str = "FavoriteShortcut_ShowMainWindow";

const LAUNCHER_HOTKEY_ID: i32 = 0xA731;
const CLIPBOARD_HOTKEY_ID: i32 = 0xA732;
const MOD_NOREPEAT: u32 = 0x4000;

/// 検証用に保存場所を変えて動かしているときは、名前を分けて本物のアプリと干渉しないようにする。
fn instance_suffix() -> String {
    if crate::paths::is_overridden_by_environment() {
        use sha1::Digest;
        let dir = crate::paths::data_directory().to_string_lossy().to_lowercase();
        let hash = sha1::Sha1::digest(dir.as_bytes());
        format!(".{:02x}{:02x}{:02x}{:02x}", hash[0], hash[1], hash[2], hash[3])
    } else {
        String::new()
    }
}

// ------------------------------------------------------------- 二重起動の防止

pub struct SingleInstance {
    handle: HANDLE,
}

impl SingleInstance {
    /// 最初のインスタンスなら Some。すでに動いていれば None。
    pub fn acquire() -> Option<Self> {
        let name = wide(&format!("{MUTEX_NAME}{}", instance_suffix()));
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }.ok()?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }
        Some(Self { handle })
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

fn show_message_id() -> u32 {
    let name = wide(&format!("{SHOW_MESSAGE_NAME}{}", instance_suffix()));
    unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
}

/// すでに起動しているインスタンスへウィンドウ表示を依頼する（C# 版にも届く）。
pub fn broadcast_show_request() {
    let id = show_message_id();
    if id != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND_BROADCAST), id, WPARAM(0), LPARAM(0));
        }
    }
}

// ---------------------------------------------------------- ホットキーの窓口

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformEvent {
    LauncherHotKey,
    ClipboardHotKey,
    ShowRequested,
}

type Handler = Box<dyn Fn(PlatformEvent) + Send + Sync>;
static HANDLER: Mutex<Option<Handler>> = Mutex::new(None);
static SHOW_MESSAGE: Mutex<u32> = Mutex::new(0);

/// グローバルホットキーと「ウィンドウを出して」の通知を受ける非表示のウィンドウ。
///
/// メッセージ専用ウィンドウ（HWND_MESSAGE）はブロードキャストを受け取れないため、
/// 非表示のトップレベルウィンドウとして作る（WS_EX_TOOLWINDOW なので Alt+Tab やタスクバーには出ない）。
pub struct HotKeyWindow {
    hwnd: HWND,
    launcher: Option<(i32, i32)>,
    clipboard: Option<(i32, i32)>,
}

impl HotKeyWindow {
    pub fn create(handler: impl Fn(PlatformEvent) + Send + Sync + 'static) -> Option<Self> {
        *HANDLER.lock().ok()? = Some(Box::new(handler));
        *SHOW_MESSAGE.lock().ok()? = show_message_id();

        unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let class_name = w!("FavoriteShortcutRsHotKeyWindow");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(hotkey_wndproc),
                hInstance: instance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name,
                w!("FavoriteShortcutRsHotKeyWindow"),
                WS_POPUP,
                -32000,
                -32000,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .ok()?;
            Some(Self { hwnd, launcher: None, clipboard: None })
        }
    }

    /// ランチャーのホットキーを登録し直す。他のアプリが使っていれば false。
    pub fn register_launcher(&mut self, modifiers: i32, key: i32) -> bool {
        let ok = self.register(LAUNCHER_HOTKEY_ID, modifiers, key, "グローバルホットキー");
        self.launcher = ok.then_some((modifiers, key));
        ok
    }

    /// 「クリップボードから登録」のホットキーを登録し直す。未設定なら解除だけして true。
    pub fn register_clipboard(&mut self, modifiers: i32, key: i32) -> bool {
        if key == 0 {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), CLIPBOARD_HOTKEY_ID);
            }
            self.clipboard = None;
            return true;
        }
        let ok = self.register(CLIPBOARD_HOTKEY_ID, modifiers, key, "「クリップボードから登録」のホットキー");
        self.clipboard = ok.then_some((modifiers, key));
        ok
    }

    pub fn launcher_hotkey(&self) -> Option<(i32, i32)> {
        self.launcher
    }

    fn register(&mut self, id: i32, modifiers: i32, key: i32, purpose: &str) -> bool {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), id);
        }
        if key == 0 {
            return false;
        }
        let vk = keys::vk_from_wpf(key);
        if vk == 0 {
            return false;
        }
        let mods = HOT_KEY_MODIFIERS(keys::win32_modifiers(modifiers) | MOD_NOREPEAT);
        let ok = unsafe { RegisterHotKey(Some(self.hwnd), id, mods, vk) }.is_ok();
        let text = keys::describe(modifiers, key);
        if ok {
            crate::log::info(&format!("{purpose}を登録しました: {text}"));
        } else {
            crate::log::warn(&format!("{purpose}を登録できませんでした: {text}"));
        }
        ok
    }
}

impl Drop for HotKeyWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), LAUNCHER_HOTKEY_ID);
            let _ = UnregisterHotKey(Some(self.hwnd), CLIPBOARD_HOTKEY_ID);
        }
    }
}

unsafe extern "system" fn hotkey_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let event = if msg == WM_HOTKEY && wparam.0 as i32 == LAUNCHER_HOTKEY_ID {
        Some(PlatformEvent::LauncherHotKey)
    } else if msg == WM_HOTKEY && wparam.0 as i32 == CLIPBOARD_HOTKEY_ID {
        Some(PlatformEvent::ClipboardHotKey)
    } else if SHOW_MESSAGE.lock().map(|m| *m != 0 && *m == msg).unwrap_or(false) {
        Some(PlatformEvent::ShowRequested)
    } else {
        None
    };

    if let Some(e) = event {
        if let Ok(guard) = HANDLER.lock()
            && let Some(h) = guard.as_ref()
        {
            h(e);
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 押されている Windows キー（設定画面でホットキーを受け付けるときに使う。egui からは取れないため）。
pub fn windows_key_down() -> bool {
    unsafe { (GetKeyState(VK_LWIN.0 as i32) as u16 & 0x8000) != 0 || (GetKeyState(VK_RWIN.0 as i32) as u16 & 0x8000) != 0 }
}

// ------------------------------------------------------------ メッセージボックス

#[derive(Clone, Copy)]
pub enum MsgIcon {
    Info,
    Warning,
    Error,
    Question,
}

fn icon_style(icon: MsgIcon) -> MESSAGEBOX_STYLE {
    match icon {
        MsgIcon::Info => MB_ICONINFORMATION,
        MsgIcon::Warning => MB_ICONWARNING,
        MsgIcon::Error => MB_ICONERROR,
        MsgIcon::Question => MB_ICONQUESTION,
    }
}

/// Windows 標準のメッセージボックス（C# 版の MessageBox.Show と同じ見た目）。
pub fn message(owner: Option<HWND>, text: &str, title: &str, icon: MsgIcon) {
    let t = wide(text);
    let c = wide(title);
    unsafe {
        MessageBoxW(owner, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | icon_style(icon));
    }
}

/// OK / キャンセルの確認。OK なら true。
pub fn confirm(owner: Option<HWND>, text: &str, title: &str, icon: MsgIcon) -> bool {
    let t = wide(text);
    let c = wide(title);
    unsafe { MessageBoxW(owner, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OKCANCEL | icon_style(icon)) == IDOK }
}

// ------------------------------------------------------------------ ウィンドウ

/// 他のアプリを操作中にホットキーで呼ばれたウィンドウを前面に出す。
pub fn force_foreground(hwnd: HWND) {
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
}

/// ウィンドウを前面に出さずに（キーボードの入力先を変えずに）表示する。
pub fn show_without_activating(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{SW_SHOWNOACTIVATE, ShowWindow};
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

/// ダイアログを開いている間、持ち主のウィンドウを操作できないようにする（モーダル表示）。
pub fn enable_window(hwnd: HWND, enabled: bool) {
    unsafe {
        let _ = EnableWindow(hwnd, enabled);
    }
}

/// マウスカーソルのあるモニターの作業領域（物理ピクセル）。
pub fn cursor_monitor_work_area() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).ok()?;
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let r = info.rcWork;
        Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
    }
}

/// ウィンドウの外枠の位置と大きさ（物理ピクセル）: (左, 上, 右, 下)。
pub fn window_rect(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).ok()? };
    Some((r.left, r.top, r.right, r.bottom))
}

/// ウィンドウがあるモニターの作業領域（タスクバーを除く）: (左, 上, 幅, 高さ)。
pub fn monitor_work_area_of(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let r = info.rcWork;
        Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
    }
}

// ------------------------------------------------------------------ メモリマップ

/// 読み取り専用でメモリマップしたファイル（フォントなど大きなファイルを、丸ごと読み込まずに参照する）。
/// 中身は使う部分だけを Windows が読み、アプリのメモリ（プライベート）としては数えられない。
pub struct MappedFile {
    ptr: *const u8,
    len: usize,
    mapping: HANDLE,
}

// 読み取り専用のビューで、閉じるまで中身は変わらないので、スレッド間で共有してよい
unsafe impl Send for MappedFile {}
unsafe impl Sync for MappedFile {}

impl MappedFile {
    pub fn open(path: &std::path::Path) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::System::Memory::{CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY};
        let file = std::fs::File::open(path).ok()?;
        let len = usize::try_from(file.metadata().ok()?.len()).ok()?;
        if len == 0 {
            return None;
        }
        unsafe {
            // マッピングがファイルを参照し続けるので、ファイルのハンドルはここで閉じてよい
            let mapping = CreateFileMappingW(HANDLE(file.as_raw_handle()), None, PAGE_READONLY, 0, 0, PCWSTR::null()).ok()?;
            let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
            if view.Value.is_null() {
                let _ = CloseHandle(mapping);
                return None;
            }
            Some(Self { ptr: view.Value as *const u8, len, mapping })
        }
    }
}

impl AsRef<[u8]> for MappedFile {
    fn as_ref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for MappedFile {
    fn drop(&mut self) {
        use windows::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
        unsafe {
            let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.ptr as *mut _ });
            let _ = CloseHandle(self.mapping);
        }
    }
}

// ------------------------------------------------------------------ 自動起動

const RUN_KEY_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// C# 版と同じ値の名前。どちらか一方の EXE だけが自動起動するように共有する。
const RUN_VALUE_NAME: &str = "FavoriteShortcut";
/// 自動起動時は --tray を付け、ウィンドウを出さずトレイ常駐で始める。
pub const TRAY_ARGUMENT: &str = "--tray";

pub fn startup_enabled() -> bool {
    read_run_value().is_some_and(|v| !v.is_empty())
}

fn read_run_value() -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let key = wide(RUN_KEY_PATH);
    let name = wide(RUN_VALUE_NAME);
    let mut size: u32 = 0;
    unsafe {
        if RegGetValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_SZ, None, None, Some(&mut size)).is_err()
        {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut size2 = (buf.len() * 2) as u32;
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut std::ffi::c_void),
            Some(&mut size2),
        )
        .ok()
        .ok()?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// 自動起動の設定を適用する（現在のユーザーの Run キーだけを操作する）。成功したら true。
pub fn set_startup_enabled(enabled: bool) -> bool {
    use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, REG_SZ, RegCloseKey, RegCreateKeyW, RegDeleteValueW, RegSetValueExW};
    // 開発・確認用に保存場所を環境変数で指定しているときは、普段使いの自動起動の登録に触れない
    if crate::paths::is_overridden_by_environment() {
        crate::log::warn("開発用の保存場所で起動しているため、自動起動の設定は変更しません。");
        return false;
    }
    let key_path = wide(RUN_KEY_PATH);
    let name = wide(RUN_VALUE_NAME);
    unsafe {
        let mut key = HKEY::default();
        if RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(key_path.as_ptr()), &mut key).is_err() {
            crate::log::warn("自動起動設定の変更に失敗しました。");
            return false;
        }
        let ok = if enabled {
            let Ok(exe) = std::env::current_exe() else {
                let _ = RegCloseKey(key);
                return false;
            };
            let value = wide(&format!("\"{}\" {TRAY_ARGUMENT}", exe.display()));
            let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
            RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)).is_ok()
        } else {
            let r = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
            r.is_ok() || r.0 == 2 // ERROR_FILE_NOT_FOUND（もともと無い）
        };
        let _ = RegCloseKey(key);
        if !ok {
            crate::log::warn("自動起動設定の変更に失敗しました。");
        }
        ok
    }
}

// ---------------------------------------------------------------- クリップボード

/// クリップボードの内容（エクスプローラーでコピーしたファイル一覧、または文字列）。
pub enum ClipboardContent {
    Files(Vec<String>),
    Text(String),
    Empty,
}

pub fn read_clipboard() -> ClipboardContent {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    const CF_HDROP: u32 = 15;
    const CF_UNICODETEXT: u32 = 13;

    unsafe {
        // 他のアプリがクリップボードを使用中だと開けないことがあるので、少しだけ待って試す
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !opened {
            crate::log::warn("クリップボードを読めませんでした。");
            return ClipboardContent::Empty;
        }

        let mut result = ClipboardContent::Empty;
        if IsClipboardFormatAvailable(CF_HDROP).is_ok()
            && let Ok(h) = GetClipboardData(CF_HDROP)
        {
            let drop = HDROP(h.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut files = Vec::new();
            for i in 0..count {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let n = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                let path = String::from_utf16_lossy(&buf[..n]);
                if !path.trim().is_empty() {
                    files.push(path);
                }
            }
            if !files.is_empty() {
                result = ClipboardContent::Files(files);
            }
        }
        if matches!(result, ClipboardContent::Empty)
            && IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok()
            && let Ok(h) = GetClipboardData(CF_UNICODETEXT)
        {
            let hglobal = windows::Win32::Foundation::HGLOBAL(h.0);
            let ptr = GlobalLock(hglobal) as *const u16;
            if !ptr.is_null() {
                let mut len = 0;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                let _ = GlobalUnlock(hglobal);
                result = ClipboardContent::Text(text);
            }
        }
        let _ = CloseClipboard();
        result
    }
}

/// 文字列をクリップボードへコピーする。
pub fn write_clipboard_text(text: &str) -> bool {
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;

    let data = wide(text);
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = (|| {
            let hmem = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2).ok()?;
            let ptr = GlobalLock(hmem) as *mut u16;
            if ptr.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            let _ = GlobalUnlock(hmem);
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0))).ok()?;
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

/// クリップボードから、ショートカットとして登録できそうなもの（C# 版 ClipboardTargets と同じ規則）。
pub fn clipboard_targets() -> Vec<String> {
    match read_clipboard() {
        ClipboardContent::Files(files) => files,
        ClipboardContent::Text(text) => from_clipboard_text(&text).into_iter().collect(),
        ClipboardContent::Empty => Vec::new(),
    }
}

/// 文字列の 1 行目が URL かパスなら、それを返す。
pub fn from_clipboard_text(text: &str) -> Option<String> {
    use crate::model::TargetType;
    const MAX_LENGTH: usize = 2048;

    let first = text.replace('\r', "");
    let mut line = first.split('\n').next().unwrap_or("").trim().to_string();

    // エクスプローラーの「パスのコピー」は前後に引用符が付く
    if line.len() >= 2 && line.starts_with('"') && line.ends_with('"') {
        line = line[1..line.len() - 1].trim().to_string();
    }
    let len = line.encode_utf16().count();
    if len == 0 || len > MAX_LENGTH {
        return None;
    }

    // パス（C:\... / \\server\...）は空白を含んでよい
    let b = line.as_bytes();
    let path_like =
        (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || line.starts_with("\\\\");
    if path_like {
        return Some(line);
    }

    // それ以外は空白を含まないものだけ（普通の文章を URL と取り違えないように）
    if line.chars().any(char::is_whitespace) {
        return None;
    }

    let kind = crate::target::detect(&line);
    if kind == TargetType::Unknown {
        return None;
    }
    // "memo:abc" のように「〇〇:」で始まるだけの文字列は URL とみなさない
    if kind == TargetType::Web && line.contains(':') && !line.contains("://") {
        return None;
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_text_rules_match_csharp() {
        assert_eq!(from_clipboard_text("https://example.com/\nsecond line").as_deref(), Some("https://example.com/"));
        assert_eq!(from_clipboard_text("\"C:\\My Files\\a.txt\"").as_deref(), Some("C:\\My Files\\a.txt"));
        assert_eq!(from_clipboard_text("\\\\server\\share\\資料").as_deref(), Some("\\\\server\\share\\資料"));
        assert_eq!(from_clipboard_text("example.com").as_deref(), Some("example.com"));
        assert_eq!(from_clipboard_text("これは普通の文章です"), None);
        assert_eq!(from_clipboard_text("hello world.com"), None);
        assert_eq!(from_clipboard_text("memo:abc"), None);
        assert_eq!(from_clipboard_text(""), None);
    }

    #[test]
    fn mapped_file_reads_the_same_bytes_and_releases_the_file() {
        let dir = std::env::temp_dir().join(format!("fsc-map-{}", crate::model::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.bin");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        {
            let mapped = MappedFile::open(&path).unwrap();
            assert_eq!(mapped.as_ref(), &data[..]);
        }
        // 閉じたあとはファイルを消せる（ハンドルを残していない）
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn empty_or_missing_files_are_not_mapped() {
        let dir = std::env::temp_dir().join(format!("fsc-map-{}", crate::model::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.bin");
        std::fs::write(&empty, b"").unwrap();
        assert!(MappedFile::open(&empty).is_none());
        assert!(MappedFile::open(&dir.join("missing.bin")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

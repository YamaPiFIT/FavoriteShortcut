//! ホットキーの表現。設定は C# 版と共通なので、WPF の Key / ModifierKeys の値で保存する。
//! ここでは WPF の値 ⇔ Win32 の仮想キーコード ⇔ 表示名 を対応付ける。

/// WPF の ModifierKeys の値。
pub const MOD_ALT: i32 = 1;
pub const MOD_CONTROL: i32 = 2;
pub const MOD_SHIFT: i32 = 4;
pub const MOD_WINDOWS: i32 = 8;

/// (WPF の Key の値, 仮想キーコード, WPF の Key の名前)
const TABLE: &[(i32, u32, &str)] = &[
    (2, 0x08, "Back"),
    (3, 0x09, "Tab"),
    (6, 0x0D, "Return"),
    (7, 0x13, "Pause"),
    (8, 0x14, "Capital"),
    (9, 0x15, "KanaMode"),
    (12, 0x19, "KanjiMode"),
    (13, 0x1B, "Escape"),
    (14, 0x1C, "ImeConvert"),
    (15, 0x1D, "ImeNonConvert"),
    (18, 0x20, "Space"),
    (19, 0x21, "PageUp"),
    (20, 0x22, "PageDown"),
    (21, 0x23, "End"),
    (22, 0x24, "Home"),
    (23, 0x25, "Left"),
    (24, 0x26, "Up"),
    (25, 0x27, "Right"),
    (26, 0x28, "Down"),
    (30, 0x2C, "PrintScreen"),
    (31, 0x2D, "Insert"),
    (32, 0x2E, "Delete"),
    (34, 0x30, "D0"),
    (35, 0x31, "D1"),
    (36, 0x32, "D2"),
    (37, 0x33, "D3"),
    (38, 0x34, "D4"),
    (39, 0x35, "D5"),
    (40, 0x36, "D6"),
    (41, 0x37, "D7"),
    (42, 0x38, "D8"),
    (43, 0x39, "D9"),
    (44, 0x41, "A"),
    (45, 0x42, "B"),
    (46, 0x43, "C"),
    (47, 0x44, "D"),
    (48, 0x45, "E"),
    (49, 0x46, "F"),
    (50, 0x47, "G"),
    (51, 0x48, "H"),
    (52, 0x49, "I"),
    (53, 0x4A, "J"),
    (54, 0x4B, "K"),
    (55, 0x4C, "L"),
    (56, 0x4D, "M"),
    (57, 0x4E, "N"),
    (58, 0x4F, "O"),
    (59, 0x50, "P"),
    (60, 0x51, "Q"),
    (61, 0x52, "R"),
    (62, 0x53, "S"),
    (63, 0x54, "T"),
    (64, 0x55, "U"),
    (65, 0x56, "V"),
    (66, 0x57, "W"),
    (67, 0x58, "X"),
    (68, 0x59, "Y"),
    (69, 0x5A, "Z"),
    (72, 0x5D, "Apps"),
    (74, 0x60, "NumPad0"),
    (75, 0x61, "NumPad1"),
    (76, 0x62, "NumPad2"),
    (77, 0x63, "NumPad3"),
    (78, 0x64, "NumPad4"),
    (79, 0x65, "NumPad5"),
    (80, 0x66, "NumPad6"),
    (81, 0x67, "NumPad7"),
    (82, 0x68, "NumPad8"),
    (83, 0x69, "NumPad9"),
    (84, 0x6A, "Multiply"),
    (85, 0x6B, "Add"),
    (86, 0x6C, "Separator"),
    (87, 0x6D, "Subtract"),
    (88, 0x6E, "Decimal"),
    (89, 0x6F, "Divide"),
    (90, 0x70, "F1"),
    (91, 0x71, "F2"),
    (92, 0x72, "F3"),
    (93, 0x73, "F4"),
    (94, 0x74, "F5"),
    (95, 0x75, "F6"),
    (96, 0x76, "F7"),
    (97, 0x77, "F8"),
    (98, 0x78, "F9"),
    (99, 0x79, "F10"),
    (100, 0x7A, "F11"),
    (101, 0x7B, "F12"),
    (102, 0x7C, "F13"),
    (103, 0x7D, "F14"),
    (104, 0x7E, "F15"),
    (105, 0x7F, "F16"),
    (106, 0x80, "F17"),
    (107, 0x81, "F18"),
    (108, 0x82, "F19"),
    (109, 0x83, "F20"),
    (110, 0x84, "F21"),
    (111, 0x85, "F22"),
    (112, 0x86, "F23"),
    (113, 0x87, "F24"),
    (114, 0x90, "NumLock"),
    (115, 0x91, "Scroll"),
    (140, 0xBA, "OemSemicolon"),
    (141, 0xBB, "OemPlus"),
    (142, 0xBC, "OemComma"),
    (143, 0xBD, "OemMinus"),
    (144, 0xBE, "OemPeriod"),
    (145, 0xBF, "OemQuestion"),
    (146, 0xC0, "OemTilde"),
    (149, 0xDB, "OemOpenBrackets"),
    (150, 0xDC, "OemPipe"),
    (151, 0xDD, "OemCloseBrackets"),
    (152, 0xDE, "OemQuotes"),
    (153, 0xDF, "Oem8"),
    (154, 0xE2, "OemBackslash"),
];

/// WPF の Key の値から仮想キーコードへ。対応していなければ 0。
pub fn vk_from_wpf(key: i32) -> u32 {
    TABLE.iter().find(|(k, _, _)| *k == key).map(|(_, vk, _)| *vk).unwrap_or(0)
}

/// 仮想キーコードから WPF の Key の値へ。対応していなければ 0（Key.None）。
pub fn wpf_from_vk(vk: u32) -> i32 {
    TABLE.iter().find(|(_, v, _)| *v == vk).map(|(k, _, _)| *k).unwrap_or(0)
}

fn name_of(key: i32) -> String {
    TABLE.iter().find(|(k, _, _)| *k == key).map(|(_, _, n)| n.to_string()).unwrap_or_else(|| key.to_string())
}

/// "Ctrl + Space" のような表示（C# 版 HotKeyService.Describe と同じ書き方）。
pub fn describe(modifiers: i32, key: i32) -> String {
    if key == 0 {
        return crate::loc::t("Str.HotKey.NotSet");
    }
    let mut parts = Vec::new();
    if modifiers & MOD_CONTROL != 0 {
        parts.push("Ctrl".to_string());
    }
    if modifiers & MOD_SHIFT != 0 {
        parts.push("Shift".to_string());
    }
    if modifiers & MOD_ALT != 0 {
        parts.push("Alt".to_string());
    }
    if modifiers & MOD_WINDOWS != 0 {
        parts.push("Win".to_string());
    }
    parts.push(name_of(key));
    parts.join(" + ")
}

/// WPF の ModifierKeys から RegisterHotKey 用の修飾キーへ。
pub fn win32_modifiers(modifiers: i32) -> u32 {
    const W_ALT: u32 = 0x0001;
    const W_CONTROL: u32 = 0x0002;
    const W_SHIFT: u32 = 0x0004;
    const W_WIN: u32 = 0x0008;
    let mut m = 0;
    if modifiers & MOD_ALT != 0 {
        m |= W_ALT;
    }
    if modifiers & MOD_CONTROL != 0 {
        m |= W_CONTROL;
    }
    if modifiers & MOD_SHIFT != 0 {
        m |= W_SHIFT;
    }
    if modifiers & MOD_WINDOWS != 0 {
        m |= W_WIN;
    }
    m
}

/// egui のキーから仮想キーコードへ（設定画面でキーを受け付けるときに使う）。
pub fn vk_from_egui(key: egui::Key) -> u32 {
    use egui::Key as K;
    match key {
        K::Space => 0x20,
        K::Enter => 0x0D,
        K::Tab => 0x09,
        K::Backspace => 0x08,
        K::Escape => 0x1B,
        K::Insert => 0x2D,
        K::Delete => 0x2E,
        K::Home => 0x24,
        K::End => 0x23,
        K::PageUp => 0x21,
        K::PageDown => 0x22,
        K::ArrowLeft => 0x25,
        K::ArrowUp => 0x26,
        K::ArrowRight => 0x27,
        K::ArrowDown => 0x28,
        K::Num0 => 0x30,
        K::Num1 => 0x31,
        K::Num2 => 0x32,
        K::Num3 => 0x33,
        K::Num4 => 0x34,
        K::Num5 => 0x35,
        K::Num6 => 0x36,
        K::Num7 => 0x37,
        K::Num8 => 0x38,
        K::Num9 => 0x39,
        K::A => 0x41,
        K::B => 0x42,
        K::C => 0x43,
        K::D => 0x44,
        K::E => 0x45,
        K::F => 0x46,
        K::G => 0x47,
        K::H => 0x48,
        K::I => 0x49,
        K::J => 0x4A,
        K::K => 0x4B,
        K::L => 0x4C,
        K::M => 0x4D,
        K::N => 0x4E,
        K::O => 0x4F,
        K::P => 0x50,
        K::Q => 0x51,
        K::R => 0x52,
        K::S => 0x53,
        K::T => 0x54,
        K::U => 0x55,
        K::V => 0x56,
        K::W => 0x57,
        K::X => 0x58,
        K::Y => 0x59,
        K::Z => 0x5A,
        K::F1 => 0x70,
        K::F2 => 0x71,
        K::F3 => 0x72,
        K::F4 => 0x73,
        K::F5 => 0x74,
        K::F6 => 0x75,
        K::F7 => 0x76,
        K::F8 => 0x77,
        K::F9 => 0x78,
        K::F10 => 0x79,
        K::F11 => 0x7A,
        K::F12 => 0x7B,
        K::Semicolon => 0xBA,
        K::Plus | K::Equals => 0xBB,
        K::Comma => 0xBC,
        K::Minus => 0xBD,
        K::Period => 0xBE,
        K::Slash | K::Questionmark => 0xBF,
        K::Backtick => 0xC0,
        K::OpenBracket => 0xDB,
        K::Backslash | K::Pipe => 0xDC,
        K::CloseBracket => 0xDD,
        K::Quote => 0xDE,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkey_is_ctrl_space() {
        // C# 版の既定（ModifierKeys.Control = 2, Key.Space = 18）
        assert_eq!(vk_from_wpf(18), 0x20);
        assert_eq!(describe(2, 18), "Ctrl + Space");
        assert_eq!(wpf_from_vk(0x41), 44);
    }

    #[test]
    fn table_has_no_duplicates() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                assert_ne!(a.0, b.0, "WPF の値が重複: {}", a.2);
                assert_ne!(a.1, b.1, "仮想キーコードが重複: {}", a.2);
            }
        }
    }
}

//! 設定の読み書き（C# 版 SettingsService と同じキー・同じ値の形式）。
//! 未知のキーや壊れた値があっても既定値で起動できるようにしている。

use std::collections::HashMap;

use crate::db::DbResult;
use crate::model::{AppLanguage, AppSettings, AppTheme, ViewMode};
use crate::store::Store;

pub fn load(store: &Store) -> AppSettings {
    let raw = store.load_settings_raw().unwrap_or_default();
    from_raw(&raw)
}

pub fn from_raw(raw: &HashMap<String, String>) -> AppSettings {
    let d = AppSettings::default();
    AppSettings {
        view_mode: match get_enum(raw, "view_mode", VIEW_MODE_NAMES, d.view_mode as i32) {
            1 => ViewMode::List,
            _ => ViewMode::Card,
        },
        icon_size: get_int(raw, "icon_size", d.icon_size).clamp(16, 96),
        theme: match get_enum(raw, "theme", THEME_NAMES, d.theme as i32) {
            0 => AppTheme::Light,
            1 => AppTheme::Dark,
            _ => AppTheme::Auto,
        },
        language: match get_enum(raw, "language", LANGUAGE_NAMES, d.language as i32) {
            1 => AppLanguage::Japanese,
            2 => AppLanguage::English,
            _ => AppLanguage::Auto,
        },
        hotkey_modifiers: get_int(raw, "hotkey_modifiers", d.hotkey_modifiers),
        hotkey_key: get_int(raw, "hotkey_key", d.hotkey_key),
        clipboard_hotkey_modifiers: get_int(raw, "clipboard_hotkey_modifiers", d.clipboard_hotkey_modifiers),
        clipboard_hotkey_key: get_int(raw, "clipboard_hotkey_key", d.clipboard_hotkey_key),
        minimize_to_tray: get_bool(raw, "minimize_to_tray", d.minimize_to_tray),
        start_minimized: get_bool(raw, "start_minimized", d.start_minimized),
        run_at_startup: get_bool(raw, "run_at_startup", d.run_at_startup),
        use_browser_icon_cache: get_bool(raw, "browser_icon_cache", d.use_browser_icon_cache),
        show_recent_in_launcher: get_bool(raw, "launcher_show_recent", d.show_recent_in_launcher),
        launcher_max_results: get_int(raw, "launcher_max_results", d.launcher_max_results).clamp(3, 50),
        restore_last_folder: get_bool(raw, "restore_last_folder", d.restore_last_folder),
        auto_backup: get_bool(raw, "auto_backup", d.auto_backup),
        last_folder_id: raw.get("last_folder_id").filter(|v| !v.is_empty()).cloned(),
        main_window_width: get_double(raw, "window_width", d.main_window_width),
        main_window_height: get_double(raw, "window_height", d.main_window_height),
        folder_pane_width: get_double(raw, "folder_pane_width", d.folder_pane_width),
    }
}

pub fn to_raw(s: &AppSettings) -> Vec<(String, String)> {
    let b = |v: bool| if v { "1".to_string() } else { "0".to_string() };
    vec![
        ("view_mode".into(), (s.view_mode as i32).to_string()),
        ("icon_size".into(), s.icon_size.to_string()),
        ("theme".into(), (s.theme as i32).to_string()),
        ("language".into(), (s.language as i32).to_string()),
        ("hotkey_modifiers".into(), s.hotkey_modifiers.to_string()),
        ("hotkey_key".into(), s.hotkey_key.to_string()),
        ("clipboard_hotkey_modifiers".into(), s.clipboard_hotkey_modifiers.to_string()),
        ("clipboard_hotkey_key".into(), s.clipboard_hotkey_key.to_string()),
        ("minimize_to_tray".into(), b(s.minimize_to_tray)),
        ("start_minimized".into(), b(s.start_minimized)),
        ("run_at_startup".into(), b(s.run_at_startup)),
        ("browser_icon_cache".into(), b(s.use_browser_icon_cache)),
        ("launcher_show_recent".into(), b(s.show_recent_in_launcher)),
        ("launcher_max_results".into(), s.launcher_max_results.to_string()),
        ("restore_last_folder".into(), b(s.restore_last_folder)),
        ("auto_backup".into(), b(s.auto_backup)),
        ("last_folder_id".into(), s.last_folder_id.clone().unwrap_or_default()),
        ("window_width".into(), format_double(s.main_window_width)),
        ("window_height".into(), format_double(s.main_window_height)),
        ("folder_pane_width".into(), format_double(s.folder_pane_width)),
    ]
}

pub fn save(store: &Store, settings: &AppSettings) -> DbResult<()> {
    store.save_settings_raw(&to_raw(settings))
}

/// C# の double.ToString(InvariantCulture) に近い表記（整数なら小数点なし）。
fn format_double(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

fn get_int(raw: &HashMap<String, String>, key: &str, fallback: i32) -> i32 {
    raw.get(key).and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(fallback)
}

fn get_double(raw: &HashMap<String, String>, key: &str, fallback: f64) -> f64 {
    raw.get(key).and_then(|v| v.trim().parse::<f64>().ok()).filter(|d| *d > 0.0).unwrap_or(fallback)
}

fn get_bool(raw: &HashMap<String, String>, key: &str, fallback: bool) -> bool {
    match raw.get(key) {
        Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        None => fallback,
    }
}

/// 数値、または C# の列挙名（"List" など）で保存された値を読む。
fn get_enum(raw: &HashMap<String, String>, key: &str, names: &[(&str, i32)], fallback: i32) -> i32 {
    let Some(v) = raw.get(key) else { return fallback };
    let v = v.trim();
    if let Ok(i) = v.parse::<i32>() {
        return i;
    }
    names.iter().find(|(name, _)| name.eq_ignore_ascii_case(v)).map(|(_, value)| *value).unwrap_or(fallback)
}

const VIEW_MODE_NAMES: &[(&str, i32)] = &[("Card", 0), ("List", 1)];
const THEME_NAMES: &[(&str, i32)] = &[("Light", 0), ("Dark", 1), ("Auto", 2)];
const LANGUAGE_NAMES: &[(&str, i32)] = &[("Auto", 0), ("Japanese", 1), ("English", 2)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_keys() {
        let mut s = AppSettings::default();
        s.view_mode = ViewMode::List;
        s.theme = AppTheme::Dark;
        s.language = AppLanguage::English;
        s.hotkey_modifiers = 6;
        s.hotkey_key = 44;
        s.last_folder_id = Some("abc".into());
        s.main_window_width = 1234.5;
        let raw: HashMap<String, String> = to_raw(&s).into_iter().collect();
        assert_eq!(from_raw(&raw), s);
    }

    #[test]
    fn broken_values_fall_back_to_defaults() {
        let raw: HashMap<String, String> = [
            ("icon_size".to_string(), "abc".to_string()),
            ("launcher_max_results".to_string(), "999".to_string()),
            ("window_width".to_string(), "-5".to_string()),
        ]
        .into_iter()
        .collect();
        let s = from_raw(&raw);
        assert_eq!(s.icon_size, 32);
        assert_eq!(s.launcher_max_results, 50);
        assert_eq!(s.main_window_width, 1040.0);
    }

    #[test]
    fn csharp_enum_names_are_understood() {
        let raw: HashMap<String, String> =
            [("view_mode".to_string(), "List".to_string()), ("theme".to_string(), "Auto".to_string())].into_iter().collect();
        let s = from_raw(&raw);
        assert_eq!(s.view_mode, ViewMode::List);
        assert_eq!(s.theme, AppTheme::Auto);
    }
}

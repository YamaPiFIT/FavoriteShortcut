//! 表示言語の切り替えと、画面に出す文字列の取得（日本語 / 英語）。
//! 文字列の本体は C# 版から生成した対訳表（strings_table.rs）にある。

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::model::AppLanguage;
use crate::strings_table::TABLE;

struct Tables {
    ja: HashMap<&'static str, &'static str>,
    en: HashMap<&'static str, &'static str>,
}

static TABLES: OnceLock<Tables> = OnceLock::new();

/// 実際に表示している言語（Auto を解決したあとの値）: 1 = 日本語, 2 = 英語
static CURRENT: AtomicU8 = AtomicU8::new(1);

fn tables() -> &'static Tables {
    TABLES.get_or_init(|| {
        let mut ja = HashMap::with_capacity(TABLE.len());
        let mut en = HashMap::with_capacity(TABLE.len());
        for (key, j, e) in TABLE {
            ja.insert(*key, *j);
            en.insert(*key, *e);
        }
        Tables { ja, en }
    })
}

/// 実際に表示している言語（Japanese / English）。
pub fn current() -> AppLanguage {
    if CURRENT.load(Ordering::Relaxed) == 2 { AppLanguage::English } else { AppLanguage::Japanese }
}

/// 設定値（Auto を含む）を適用する。表示言語が変わったら true。
pub fn apply(setting: AppLanguage) -> bool {
    let resolved = resolve(setting);
    let value = if resolved == AppLanguage::English { 2 } else { 1 };
    CURRENT.swap(value, Ordering::Relaxed) != value
}

/// 「自動」を Windows の表示言語で解決する。日本語環境なら日本語、それ以外は英語。
pub fn resolve(setting: AppLanguage) -> AppLanguage {
    match setting {
        AppLanguage::Japanese => AppLanguage::Japanese,
        AppLanguage::English => AppLanguage::English,
        AppLanguage::Auto => {
            if system_ui_is_japanese() {
                AppLanguage::Japanese
            } else {
                AppLanguage::English
            }
        }
    }
}

/// Windows の表示言語が日本語か。
pub fn system_ui_is_japanese() -> bool {
    // LANGID の下位 10 ビットが主言語。0x11 = 日本語
    let langid = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    (langid & 0x3ff) == 0x11
}

/// Windows の表示言語の名前（「日本語 (日本)」のような、その言語自身での表記）。
pub fn system_ui_language_name() -> String {
    use windows::Win32::Globalization::{GetLocaleInfoEx, GetUserDefaultUILanguage, LCIDToLocaleName};
    use windows::core::PCWSTR;
    const LOCALE_SNATIVEDISPLAYNAME: u32 = 0x73;

    let langid = unsafe { GetUserDefaultUILanguage() };
    let mut name = [0u16; 85];
    let len = unsafe { LCIDToLocaleName(langid as u32, Some(&mut name), 0) };
    if len <= 1 {
        return String::new();
    }

    let mut display = [0u16; 128];
    let n = unsafe { GetLocaleInfoEx(PCWSTR(name.as_ptr()), LOCALE_SNATIVEDISPLAYNAME, Some(&mut display)) };
    if n > 1 { String::from_utf16_lossy(&display[..(n as usize - 1)]) } else { String::from_utf16_lossy(&name[..(len as usize - 1)]) }
}

/// キーに対応する文字列。見つからない場合はキーをそのまま返す（表示が消えないように）。
pub fn t(key: &str) -> String {
    let t = tables();
    let map = if current() == AppLanguage::English { &t.en } else { &t.ja };
    match map.get(key) {
        Some(v) => (*v).to_string(),
        None => {
            crate::log::warn(&format!("文字列リソースが見つかりません: {key}"));
            key.to_string()
        }
    }
}

/// {0} などのプレースホルダを埋めた文字列（C# の string.Format と同じ書式）。
pub fn tf(key: &str, args: &[&dyn std::fmt::Display]) -> String {
    format_placeholders(&t(key), args)
}

/// "{0}" "{1}" を順に置き換える。"{{" "}}" は波かっこそのもの。
pub fn format_placeholders(format: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(format.len() + 16);
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                out.push('{');
                i += 2;
                continue;
            }
            if let Some(close) = chars[i + 1..].iter().position(|&ch| ch == '}') {
                let inner: String = chars[i + 1..i + 1 + close].iter().collect();
                // "{0:N0}" のような書式指定は数字の部分だけ見る
                let index_part = inner.split(':').next().unwrap_or("");
                if let Ok(index) = index_part.trim().parse::<usize>()
                    && let Some(arg) = args.get(index)
                {
                    out.push_str(&arg.to_string());
                    i += close + 2;
                    continue;
                }
            }
        } else if c == '}' && i + 1 < chars.len() && chars[i + 1] == '}' {
            out.push('}');
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 「自動」で何語になるかの説明（設定画面用）。
pub fn describe_auto_detection() -> String {
    let resolved = if system_ui_is_japanese() { t("Str.Settings.LanguageNameJapanese") } else { t("Str.Settings.LanguageNameEnglish") };
    tf("Str.Settings.LanguageAutoNote", &[&system_ui_language_name(), &resolved])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_replaced() {
        assert_eq!(format_placeholders("{0} / {1}", &[&1, &"a"]), "1 / a");
        assert_eq!(format_placeholders("{{x}} {0}", &[&5]), "{x} 5");
        assert_eq!(format_placeholders("{2}", &[&1]), "{2}");
    }

    #[test]
    fn every_key_has_both_languages() {
        for (key, ja, en) in TABLE {
            assert!(!ja.is_empty(), "{key} の日本語が空です");
            assert!(!en.is_empty(), "{key} の英語が空です");
        }
    }

    #[test]
    fn english_has_no_japanese_characters() {
        for (key, _, en) in TABLE {
            let has_japanese = en.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'));
            assert!(!has_japanese, "{key} の英語に日本語が含まれています: {en}");
        }
    }
}

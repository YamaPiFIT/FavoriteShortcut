//! 文字列比較などの小物。C# 版の StringComparer.CurrentCulture / OrdinalIgnoreCase に合わせる。

use std::cmp::Ordering;

use windows::Win32::Globalization::{
    COMPARE_STRING_FLAGS, CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringEx, LINGUISTIC_IGNORECASE,
};
use windows::core::PCWSTR;

/// 利用者の地域設定に従った並び順（C# の StringComparer.CurrentCulture 相当）。
pub fn culture_cmp(a: &str, b: &str) -> Ordering {
    compare_string(a, b, COMPARE_STRING_FLAGS(0))
}

/// 大文字小文字を区別しない、地域設定に従った並び順（CurrentCultureIgnoreCase 相当）。
pub fn culture_cmp_ignore_case(a: &str, b: &str) -> Ordering {
    compare_string(a, b, LINGUISTIC_IGNORECASE)
}

fn compare_string(a: &str, b: &str, flags: COMPARE_STRING_FLAGS) -> Ordering {
    let wa: Vec<u16> = a.encode_utf16().collect();
    let wb: Vec<u16> = b.encode_utf16().collect();
    // LOCALE_NAME_USER_DEFAULT は NULL
    let result = unsafe { CompareStringEx(PCWSTR::null(), flags, &wa, &wb, None, None, None) };
    match result {
        r if r == CSTR_LESS_THAN => Ordering::Less,
        r if r == CSTR_GREATER_THAN => Ordering::Greater,
        r if r == CSTR_EQUAL => Ordering::Equal,
        _ => a.cmp(b), // 比較できない場合は単純な順序
    }
}

/// 大文字小文字を区別しない一致（C# の StringComparison.OrdinalIgnoreCase 相当）。
pub fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.eq_ignore_ascii_case(b) || fold(a) == fold(b)
}

/// 大文字小文字を区別しない部分一致。
pub fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    fold(haystack).contains(&fold(needle))
}

/// 大文字小文字を区別しない前方一致。
pub fn starts_with_ignore_case(haystack: &str, needle: &str) -> bool {
    fold(haystack).starts_with(&fold(needle))
}

/// 比較用に大文字へそろえる（.NET の OrdinalIgnoreCase は大文字化して比べる）。
pub fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_uppercase).collect()
}

/// 文字数で切り詰める（全角も 1 文字として数える）。
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Windows の API に渡す NUL 終端の UTF-16 文字列。
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_case_comparisons() {
        assert!(eq_ignore_case("Chrome", "chrome"));
        assert!(!eq_ignore_case("Chrome", "chrom"));
        assert!(contains_ignore_case("GitHub", "hub"));
        assert!(starts_with_ignore_case("GitHub", "git"));
    }

    #[test]
    fn culture_order_is_case_insensitive_when_requested() {
        assert_eq!(culture_cmp_ignore_case("apple", "Apple"), Ordering::Equal);
        assert_eq!(culture_cmp("a", "b"), Ordering::Less);
    }
}

//! 検索用の文字列正規化（C# 版 TextNormalizer と同じ規則）。
//!
//!   - 英字の大文字/小文字
//!   - 全角英数記号 → 半角（Ｇｏｏｇｌｅ → google）
//!   - 全角スペース → 半角スペース
//!   - ひらがな → カタカナ（かいはつ と カイハツ を同一視）

pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let mut c = ch;
        let code = c as u32;

        if (0xFF01..=0xFF5E).contains(&code) {
            // 全角 ASCII（！～～） → 半角
            c = char::from_u32(code - 0xFEE0).unwrap_or(c);
        } else if c == '\u{3000}' {
            // 全角スペース → 半角スペース
            c = ' ';
        } else if (0x3041..=0x3096).contains(&code) {
            // ひらがな → カタカナ（ゝゞ は対象外）
            c = char::from_u32(code + 0x60).unwrap_or(c);
        }

        if c.is_ascii_uppercase() {
            c = c.to_ascii_lowercase();
        }
        out.push(c);
    }
    out
}

/// 検索クエリを空白区切りの単語列に分解する（全角スペースも区切りとして扱う）。
pub fn split_terms(query: &str) -> Vec<String> {
    normalize(query).split(' ').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_like_csharp() {
        assert_eq!(normalize("Ｇｏｏｇｌｅ"), "google");
        assert_eq!(normalize("かいはつ"), "カイハツ");
        assert_eq!(normalize("ABC　def"), "abc def");
        assert_eq!(normalize("ゝゞ"), "ゝゞ");
    }

    #[test]
    fn splits_on_full_width_spaces() {
        assert_eq!(split_terms("  開発　ｇｉｔ  "), vec!["開発", "git"]);
        assert!(split_terms("   ").is_empty());
    }
}

using System.Text;

namespace FavoriteShortcut.Services;

/// <summary>
/// 検索用の文字列正規化。
///
/// 日本語環境で「打った文字」と「登録した文字」の細かな違いで検索が外れないよう、
/// 次を吸収する（§14 の部分一致・日本語検索要件）。
///   - 英字の大文字/小文字
///   - 全角英数記号 → 半角（Ｇｏｏｇｌｅ → google）
///   - 全角スペース → 半角スペース
///   - ひらがな → カタカナ（かいはつ と カイハツ を同一視）
/// </summary>
public static class TextNormalizer
{
    public static string Normalize(string? text)
    {
        if (string.IsNullOrEmpty(text)) return string.Empty;

        var sb = new StringBuilder(text.Length);
        foreach (var ch in text)
        {
            var c = ch;

            // 全角 ASCII（！～～） → 半角
            if (c >= '！' && c <= '～')
                c = (char)(c - 0xFEE0);
            // 全角スペース → 半角スペース
            else if (c == '　')
                c = ' ';
            // ひらがな → カタカナ（ゝゞ は対象外）
            else if (c >= 'ぁ' && c <= 'ゖ')
                c = (char)(c + 0x60);

            if (c >= 'A' && c <= 'Z')
                c = (char)(c + ('a' - 'A'));

            sb.Append(c);
        }

        return sb.ToString();
    }

    /// <summary>検索クエリを空白区切りの単語列に分解する（全角スペースも区切りとして扱う）。</summary>
    public static string[] SplitTerms(string? query)
    {
        var normalized = Normalize(query);
        if (normalized.Length == 0) return Array.Empty<string>();
        return normalized.Split(' ', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
    }
}

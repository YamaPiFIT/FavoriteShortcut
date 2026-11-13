//! URL / パス文字列の種類判定と整形（C# 版 TargetResolver と同じ規則）。
//!
//! 判定はあくまで「登録時点での推測」であり、対象が存在しない場合でも
//! 登録自体は許可する（§33：DB上の情報を勝手に消さない）。

use std::path::Path;

use crate::model::TargetType;
use crate::util::wide;

const EXECUTABLE_EXTENSIONS: &[&str] = &[".exe", ".com", ".bat", ".cmd", ".msi", ".lnk", ".ps1", ".vbs"];

/// "^[a-zA-Z][a-zA-Z0-9+.\-]*:" に一致するか。一致すればスキーム名を返す。
fn scheme_of(value: &str) -> Option<&str> {
    let mut chars = value.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    for (i, c) in chars {
        if c == ':' {
            return Some(&value[..i]);
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-') {
            return None;
        }
    }
    None
}

/// "^[a-zA-Z]:[\\/]"
fn is_drive_path(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// "^[a-zA-Z0-9\-]+(\.[a-zA-Z0-9\-]+)+(/.*)?$"（www.example.com / example.co.jp など）
fn is_bare_host(value: &str) -> bool {
    let host = value.split('/').next().unwrap_or("");
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2 && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// 入力欄の文字列から対象種別を推測する。
pub fn detect(target: &str) -> TargetType {
    let value = target.trim();
    if value.is_empty() {
        return TargetType::Unknown;
    }

    // UNC パス
    if value.starts_with("\\\\") {
        return if looks_like_file(value) { file_or_app(value) } else { TargetType::Folder };
    }

    // ドライブレターつきのローカルパス
    if is_drive_path(value) {
        let p = Path::new(value);
        if p.is_dir() {
            return TargetType::Folder;
        }
        if p.is_file() {
            return file_or_app(value);
        }
        return if looks_like_file(value) { file_or_app(value) } else { TargetType::Folder };
    }

    if let Some(scheme) = scheme_of(value) {
        return match scheme.to_ascii_lowercase().as_str() {
            "http" | "https" | "ftp" | "ftps" => TargetType::Web,
            "file" => TargetType::File,
            _ => TargetType::Web, // mailto:, ms-settings:, obsidian: などは既定ハンドラに任せる
        };
    }

    // 環境変数を含むパス（%USERPROFILE%\Documents など）
    if value.contains('%') {
        let expanded = expand(value);
        let p = Path::new(&expanded);
        if p.is_dir() {
            return TargetType::Folder;
        }
        if p.is_file() {
            return file_or_app(&expanded);
        }
    }

    if is_bare_host(value) {
        return TargetType::Web;
    }

    TargetType::Unknown
}

fn file_or_app(path: &str) -> TargetType {
    let ext = extension(path).to_ascii_lowercase();
    if EXECUTABLE_EXTENSIONS.contains(&ext.as_str()) { TargetType::Application } else { TargetType::File }
}

fn looks_like_file(value: &str) -> bool {
    let ext = extension(value);
    let len = ext.encode_utf16().count();
    len > 1 && len <= 8 && !ext.contains(' ')
}

/// 保存前の整形。"example.com" のようにスキームが無い Web アドレスには https:// を補う。
pub fn normalize(target: &str, kind: TargetType) -> String {
    let mut value = target.trim().to_string();
    if value.is_empty() {
        return value;
    }
    // 貼り付け時に前後へ付きがちな引用符を落とす
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value = value[1..value.len() - 1].trim().to_string();
    }
    if kind == TargetType::Web && scheme_of(&value).is_none() {
        value = format!("https://{value}");
    }
    value
}

/// 対象から既定のタイトル候補を作る（登録ダイアログの補完用）。
pub fn suggest_title(target: &str, kind: TargetType) -> String {
    let value = target.trim();
    if value.is_empty() {
        return String::new();
    }
    match kind {
        TargetType::Web => match url::Url::parse(value) {
            Ok(u) => {
                let host = u.host_str().unwrap_or("");
                let host = strip_www(host);
                if host.is_empty() { value.to_string() } else { host.to_string() }
            }
            Err(_) => value.to_string(),
        },
        TargetType::Folder => {
            let name = file_name(value.trim_end_matches(['\\', '/']));
            if name.is_empty() { value.to_string() } else { name.to_string() }
        }
        TargetType::File | TargetType::Application => {
            let name = file_stem(value);
            if name.is_empty() { value.to_string() } else { name.to_string() }
        }
        TargetType::Unknown => value.to_string(),
    }
}

fn strip_www(host: &str) -> &str {
    if host.len() >= 4 && host[..4].eq_ignore_ascii_case("www.") { &host[4..] } else { host }
}

/// Web URL を解釈する。スキームが無い場合だけ https:// を補う。ホストの無いものは None。
pub fn try_get_uri(target: &str) -> Option<url::Url> {
    let value = target.trim();
    if value.is_empty() {
        return None;
    }
    let value = if scheme_of(value).is_none() { format!("https://{value}") } else { value.to_string() };
    let u = url::Url::parse(&value).ok()?;
    if u.host_str().is_some_and(|h| !h.is_empty()) { Some(u) } else { None }
}

/// Web URL からホスト名を取り出す。
pub fn try_get_host(target: &str) -> Option<String> {
    try_get_uri(target).and_then(|u| u.host_str().map(str::to_string))
}

/// スキーム + ホスト + ポートを取り出す（favicon のキャッシュキー用。C# 版と同じ形）。
pub fn try_get_origin(target: &str) -> Option<String> {
    let u = try_get_uri(target)?;
    let host = u.host_str()?;
    Some(match u.port() {
        Some(port) => format!("{}://{}:{}", u.scheme(), host, port),
        None => format!("{}://{}", u.scheme(), host),
    })
}

/// 環境変数を展開した実際のパス（Web の場合はそのまま）。
pub fn expand(target: &str) -> String {
    if !target.contains('%') {
        return target.to_string();
    }
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows::core::PCWSTR;
    let src = wide(target);
    let needed = unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), None) };
    if needed == 0 {
        return target.to_string();
    }
    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), Some(&mut buf)) };
    if written == 0 {
        return target.to_string();
    }
    String::from_utf16_lossy(&buf[..(written as usize).saturating_sub(1)])
}

/// 対象が実在するか。Web は常に true（オフラインでも登録は有効なため）。
pub fn exists(target: &str, kind: TargetType) -> bool {
    if matches!(kind, TargetType::Web | TargetType::Unknown) {
        return true;
    }
    let path = expand(target);
    if kind == TargetType::Folder { Path::new(&path).is_dir() } else { Path::new(&path).is_file() }
}

/// UNC パス（\\server\share）またはネットワークドライブ上のパスか。
/// こうしたパスはサーバーの応答を待つことがあるので、画面の処理の中では読みに行かない。
pub fn is_network_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let mut p = path;
    // \\?\ や \\.\ で始まる長いパス用の書き方は、その後ろで判断する（\\?\UNC\server\share は UNC）
    if p.starts_with("\\\\?\\") || p.starts_with("\\\\.\\") {
        p = &p[4..];
        if p.len() >= 4 && p[..4].eq_ignore_ascii_case("UNC\\") {
            return true;
        }
    } else if p.starts_with("\\\\") {
        return true;
    }

    let b = p.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        use windows::Win32::Storage::FileSystem::GetDriveTypeW;
        use windows::core::PCWSTR;
        const DRIVE_REMOTE: u32 = 4;
        // ドライブの種類はドライブ自体に触らずに分かる
        let root = wide(&format!("{}:\\", b[0] as char));
        return unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == DRIVE_REMOTE;
    }
    false
}

// --------------------------------------------------------- パス文字列の小物

/// 最後の区切り（\ または /）より後ろの部分（C# の Path.GetFileName）。
pub fn file_name(path: &str) -> &str {
    match path.rfind(['\\', '/']) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// 拡張子（"." を含む。C# の Path.GetExtension と同じく末尾が "." なら空）。
pub fn extension(path: &str) -> &str {
    let name = file_name(path);
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => &name[i..],
        _ => "",
    }
}

/// 拡張子を除いたファイル名（C# の Path.GetFileNameWithoutExtension）。
pub fn file_stem(path: &str) -> &str {
    let name = file_name(path);
    match name.rfind('.') {
        Some(i) => &name[..i],
        None => name,
    }
}

/// 親フォルダのパス（C# の Path.GetDirectoryName）。
pub fn parent_dir(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(['\\', '/']);
    let i = trimmed.rfind(['\\', '/'])?;
    let parent = &trimmed[..i];
    if parent.is_empty() {
        None
    } else if parent.len() == 2 && parent.ends_with(':') {
        Some(&trimmed[..i + 1]) // "C:" ではなく "C:\"
    } else {
        Some(parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_types_like_csharp() {
        assert_eq!(detect("https://www.google.com/"), TargetType::Web);
        assert_eq!(detect("http://example.com"), TargetType::Web);
        assert_eq!(detect("example.co.jp"), TargetType::Web);
        assert_eq!(detect("\\\\server\\share\\資料"), TargetType::Folder);
        assert_eq!(detect("\\\\server\\share\\a.xlsx"), TargetType::File);
        assert_eq!(detect("\\\\server\\share\\tool.exe"), TargetType::Application);
        assert_eq!(detect("C:\\fsc-no-such\\memo.txt"), TargetType::File);
        assert_eq!(detect("mailto:a@example.com"), TargetType::Web);
        assert_eq!(detect("file:///C:/a.txt"), TargetType::File);
        assert_eq!(detect("ただの文章"), TargetType::Unknown);
        assert_eq!(detect(""), TargetType::Unknown);
    }

    #[test]
    fn missing_path_does_not_exist() {
        assert!(!exists("C:\\test\\sample.xlsx", TargetType::File));
        assert!(exists("https://example.com", TargetType::Web));
    }

    #[test]
    fn normalizes_and_suggests_titles() {
        assert_eq!(normalize("example.com", TargetType::Web), "https://example.com");
        assert_eq!(normalize("\"C:\\a b\\c.txt\"", TargetType::File), "C:\\a b\\c.txt");
        assert_eq!(suggest_title("https://www.github.com/x", TargetType::Web), "github.com");
        assert_eq!(suggest_title("C:\\仕事\\資料\\", TargetType::Folder), "資料");
        assert_eq!(suggest_title("C:\\a\\見積.xlsx", TargetType::File), "見積");
    }

    #[test]
    fn origin_keeps_scheme_and_port() {
        assert_eq!(try_get_origin("https://example.com/a/b").as_deref(), Some("https://example.com"));
        assert_eq!(try_get_origin("http://portal/").as_deref(), Some("http://portal"));
        assert_eq!(try_get_origin("https://intranet:8443/app/").as_deref(), Some("https://intranet:8443"));
        assert_ne!(try_get_origin("http://portal/"), try_get_origin("https://portal/"));
    }

    #[test]
    fn recognizes_network_paths() {
        assert!(is_network_path("\\\\server\\share\\資料"));
        assert!(is_network_path("\\\\?\\UNC\\server\\share\\a.txt"));
        assert!(!is_network_path("\\\\?\\C:\\Windows"));
        assert!(!is_network_path("C:\\Windows"));
        assert!(!is_network_path("https://example.com/"));
        assert!(!is_network_path(""));
    }

    #[test]
    fn path_helpers_match_dotnet() {
        assert_eq!(extension("C:\\a\\b.txt"), ".txt");
        assert_eq!(extension("C:\\a.b\\c"), "");
        assert_eq!(file_stem("C:\\a\\b.tar.gz"), "b.tar");
        assert_eq!(parent_dir("C:\\a\\b.txt"), Some("C:\\a"));
        assert_eq!(parent_dir("C:\\a"), Some("C:\\"));
    }
}

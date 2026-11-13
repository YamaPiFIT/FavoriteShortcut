//! Chromium 系ブラウザ（Edge / Chrome など）のお気に入りを読み取る（C# 版 BrowserBookmarkReader の移植）。
//! プロファイルフォルダ内の "Bookmarks"（JSON）を読むだけで、ブラウザ側には一切書き込まない。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::loc;
use crate::paths::{local_app_data, roaming_app_data};

/// 取り込み元として選べるブラウザのプロファイル。
#[derive(Clone, Debug)]
pub struct BookmarkProfile {
    pub browser_name: String,
    pub profile_label: String,
    pub file_path: PathBuf,
}

impl BookmarkProfile {
    /// 一覧に出す表示名。プロファイルが 1 つだけなら括弧は付けない。
    pub fn display_name(&self) -> String {
        if self.profile_label.is_empty() {
            self.browser_name.clone()
        } else {
            loc::tf("Str.Bookmark.ProfileFormat", &[&self.browser_name, &self.profile_label])
        }
    }
}

/// お気に入り 1 項目。url が None ならフォルダ。
#[derive(Clone, Debug, Default)]
pub struct BookmarkNode {
    pub name: String,
    pub url: Option<String>,
    /// 追加日時（C# 版と同じく読み込むが、画面では使っていない）
    #[allow(dead_code)]
    pub added_at: Option<DateTime<Utc>>,
    pub children: Vec<BookmarkNode>,
}

impl BookmarkNode {
    pub fn is_folder(&self) -> bool {
        self.url.is_none()
    }

    /// 配下にあるリンクの総数（フォルダは数えない）。
    pub fn link_count(&self) -> usize {
        if self.is_folder() { self.children.iter().map(BookmarkNode::link_count).sum() } else { 1 }
    }

    /// 配下にあるフォルダの総数（自分は数えない）。
    pub fn folder_count(&self) -> usize {
        self.children.iter().map(|c| if c.is_folder() { 1 + c.folder_count() } else { 0 }).sum()
    }
}

/// この PC で取り込み可能なプロファイルを探す。
pub fn find_profiles() -> Vec<BookmarkProfile> {
    let local = local_app_data();
    let roaming = roaming_app_data();
    let roots = [
        (local.join("Microsoft").join("Edge").join("User Data"), "Microsoft Edge"),
        (local.join("Google").join("Chrome").join("User Data"), "Google Chrome"),
        (local.join("BraveSoftware").join("Brave-Browser").join("User Data"), "Brave"),
        (local.join("Vivaldi").join("User Data"), "Vivaldi"),
        (local.join("Chromium").join("User Data"), "Chromium"),
        (roaming.join("Opera Software").join("Opera Stable"), "Opera"),
        (roaming.join("Opera Software").join("Opera GX Stable"), "Opera GX"),
    ];

    let mut result = Vec::new();
    for (root, browser) in roots {
        if !root.is_dir() {
            continue;
        }
        let friendly = read_profile_names(&root);

        // Opera のようにプロファイル直下に置く構成
        let direct = root.join("Bookmarks");
        if direct.is_file() {
            result.push(BookmarkProfile { browser_name: browser.into(), profile_label: String::new(), file_path: direct });
        }

        // Chrome / Edge のように Default, Profile 1 ... に分かれる構成
        let Ok(entries) = fs::read_dir(&root) else { continue };
        let mut dirs: Vec<_> = entries.flatten().collect();
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            let leaf = entry.file_name().to_string_lossy().into_owned();
            if !leaf.eq_ignore_ascii_case("Default") && !leaf.to_ascii_lowercase().starts_with("profile ") {
                continue;
            }
            let file = entry.path().join("Bookmarks");
            if !file.is_file() {
                continue;
            }
            let label = friendly.get(&leaf.to_lowercase()).cloned().unwrap_or(leaf);
            result.push(BookmarkProfile { browser_name: browser.into(), profile_label: label, file_path: file });
        }
    }
    result
}

/// Local State からプロファイルの表示名（「仕事用」など利用者が付けた名前）を読む。
fn read_profile_names(root: &Path) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let Ok(text) = read_file_safely(&root.join("Local State")) else { return names };
    let Ok(json) = serde_json::from_str::<Value>(&text) else { return names };
    if let Some(cache) = json.get("profile").and_then(|p| p.get("info_cache")).and_then(Value::as_object) {
        for (key, value) in cache {
            if let Some(name) = value.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                names.insert(key.to_lowercase(), name.to_string());
            }
        }
    }
    names
}

/// Bookmarks ファイルを読み、最上位のフォルダ（ブックマーク バー / その他 など）を返す。
pub fn read_roots(path: &Path) -> Result<Vec<BookmarkNode>, String> {
    let text = read_file_safely(path).map_err(|e| e.to_string())?;
    parse_roots(&text)
}

pub fn parse_roots(json_text: &str) -> Result<Vec<BookmarkNode>, String> {
    let json: Value = serde_json::from_str(json_text).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    let Some(roots) = json.get("roots").and_then(Value::as_object) else { return Ok(result) };

    // bookmark_bar → other → synced の順に並べる（JSON の並び順は保証されないため）
    let order = ["bookmark_bar", "other", "synced"];
    for key in order {
        if let Some(node) = roots.get(key).and_then(parse_node)
            && node.is_folder()
        {
            result.push(node);
        }
    }
    for (key, value) in roots {
        if order.contains(&key.as_str()) || !value.is_object() {
            continue;
        }
        if let Some(node) = parse_node(value)
            && node.is_folder()
        {
            result.push(node);
        }
    }
    Ok(result)
}

fn parse_node(element: &Value) -> Option<BookmarkNode> {
    let obj = element.as_object()?;
    let kind = obj.get("type").and_then(Value::as_str);
    let name = obj.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let added_at = obj.get("date_added").and_then(parse_chrome_time);

    if kind == Some("url") {
        let url = obj.get("url").and_then(Value::as_str).unwrap_or("").to_string();
        if url.trim().is_empty() {
            return None;
        }
        let lower = url.to_ascii_lowercase();
        // ブックマークレットやブラウザ内部ページはショートカットとして開けないので除外する
        if lower.starts_with("javascript:") || lower.starts_with("chrome://") || lower.starts_with("edge://") {
            return None;
        }
        let name = if name.is_empty() { url.clone() } else { name };
        return Some(BookmarkNode { name, url: Some(url), added_at, children: Vec::new() });
    }

    // type が無い場合もフォルダ（roots 直下など）として扱う
    let mut folder = BookmarkNode { name, url: None, added_at, children: Vec::new() };
    if let Some(children) = obj.get("children").and_then(Value::as_array) {
        folder.children = children.iter().filter_map(parse_node).collect();
    }
    Some(folder)
}

/// Chromium の date_added は 1601-01-01 からのマイクロ秒。壊れていれば None。
fn parse_chrome_time(value: &Value) -> Option<DateTime<Utc>> {
    let micros: i64 = match value {
        Value::String(s) => s.parse().ok()?,
        Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    if micros <= 0 {
        return None;
    }
    const EPOCH_DIFF_MICROS: i64 = 11_644_473_600_000_000; // 1601-01-01 から 1970-01-01 まで
    DateTime::from_timestamp_micros(micros - EPOCH_DIFF_MICROS)
}

/// ブラウザが起動中でも読めるようにする。直接読めない場合だけ一時コピーを経由する。
fn read_file_safely(path: &Path) -> std::io::Result<String> {
    match fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string()),
        Err(_) => {
            let temp = std::env::temp_dir().join(format!("fsc-bookmarks-{}", crate::model::new_id()));
            let result = fs::copy(path, &temp).and_then(|_| fs::read(&temp));
            let _ = fs::remove_file(&temp);
            result.map(|b| String::from_utf8_lossy(&b).trim_start_matches('\u{feff}').to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実際のブラウザではなく、同じ形式の検証用データで確かめる（利用者のブックマークには触れない）
    const SAMPLE: &str = r#"{
      "roots": {
        "other": { "type": "folder", "name": "その他", "children": [] },
        "bookmark_bar": { "type": "folder", "name": "お気に入りバー", "children": [
          { "type": "url", "name": "GitHub", "url": "https://github.com/", "date_added": "13350000000000000" },
          { "type": "url", "name": "bm", "url": "javascript:alert(1)" },
          { "type": "url", "name": "", "url": "edge://settings" },
          { "type": "folder", "name": "開発", "children": [
            { "type": "url", "name": "", "url": "https://docs.rs/" }
          ]}
        ]},
        "sync_transaction_version": "1"
      }
    }"#;

    #[test]
    fn parses_folders_and_links() {
        let roots = parse_roots(SAMPLE).unwrap();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].name, "お気に入りバー", "bookmark_bar が先");
        assert_eq!(roots[0].link_count(), 2, "javascript: と edge:// は除外");
        assert_eq!(roots[0].folder_count(), 1);
        let dev = &roots[0].children[1];
        assert_eq!(dev.children[0].name, "https://docs.rs/", "名前が無ければ URL");
        assert!(roots[0].children[0].added_at.is_some());
    }

    #[test]
    fn broken_json_is_an_error() {
        assert!(parse_roots("{ broken").is_err());
    }
}

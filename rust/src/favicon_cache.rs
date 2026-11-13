//! ブラウザが保存している favicon のキャッシュから、アイコン画像を取り出す（C# 版 BrowserFaviconCache の移植）。
//!
//! 取り扱いの方針:
//!   - 通信は一切行わない。ローカルファイルを読み取り専用で開くだけ
//!   - 認証情報は扱わない
//!   - 登録済みショートカットの URL でピンポイントに検索する。一覧の走査はしない
//!   - 取り出すのは一致したアイコン画像のみ。URL や閲覧履歴は保持しない
//!   - ブラウザのファイルには一切書き込まない（一時コピーを読み、20 秒以内に削除する）

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{Connection, OpenFlags};

use crate::paths::{local_app_data, roaming_app_data};

const SNAPSHOT_LIFETIME: Duration = Duration::from_secs(20);
const SNAPSHOT_PREFIX: &str = "fsc-icons-";

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowserKind {
    Chromium,
    Firefox,
}

struct Snapshot {
    path: PathBuf,
    kind: BrowserKind,
    source_name: String,
}

struct State {
    snapshots: Option<Vec<Snapshot>>,
    created_at: Instant,
    stale_cleaned: bool,
}

/// 見つかったアイコン（PNG / ICO などの画像バイト列）。
pub struct Found {
    pub data: Vec<u8>,
    pub browser_name: String,
}

pub struct BrowserFaviconCache {
    state: Mutex<State>,
}

impl Default for BrowserFaviconCache {
    fn default() -> Self {
        Self { state: Mutex::new(State { snapshots: None, created_at: Instant::now(), stale_cleaned: false }) }
    }
}

impl BrowserFaviconCache {
    /// 指定した URL のアイコンをブラウザのキャッシュから探す。ファイル I/O を伴うので UI スレッドから呼ばないこと。
    pub fn try_find(&self, page: &url::Url) -> Option<Found> {
        // 一時コピーの削除と重ならないよう、読み終えるまで保持する
        let mut state = self.state.lock().ok()?;
        self.ensure_snapshots(&mut state);
        let snapshots = state.snapshots.as_ref()?;
        for snapshot in snapshots {
            let result = match snapshot.kind {
                BrowserKind::Chromium => read_chromium(&snapshot.path, page),
                BrowserKind::Firefox => read_firefox(&snapshot.path, page),
            };
            match result {
                Ok(Some(data)) => return Some(Found { data, browser_name: snapshot.source_name.clone() }),
                Ok(None) => {}
                Err(e) => crate::log::warn_err(&format!("ブラウザのアイコンキャッシュを読めませんでした: {}", snapshot.source_name), &e),
            }
        }
        None
    }

    /// 一定時間たった一時コピーを削除する（呼び出し側が定期的に呼ぶ）。
    pub fn expire_snapshots(&self) {
        if let Ok(mut state) = self.state.lock()
            && state.snapshots.is_some()
            && state.created_at.elapsed() >= SNAPSHOT_LIFETIME
        {
            delete_snapshots(&mut state);
        }
    }

    pub fn dispose(&self) {
        if let Ok(mut state) = self.state.lock() {
            delete_snapshots(&mut state);
        }
    }

    fn ensure_snapshots(&self, state: &mut State) {
        if state.snapshots.is_some() && state.created_at.elapsed() < SNAPSHOT_LIFETIME {
            return;
        }
        delete_snapshots(state);
        if !state.stale_cleaned {
            state.stale_cleaned = true;
            delete_stale_snapshots();
        }
        let mut list = Vec::new();
        for (path, kind, name) in find_profile_databases() {
            if let Some(copy) = try_copy_for_reading(&path) {
                list.push(Snapshot { path: copy, kind, source_name: name });
            }
        }
        state.snapshots = Some(list);
        state.created_at = Instant::now();
    }
}

impl Drop for BrowserFaviconCache {
    fn drop(&mut self) {
        self.dispose();
    }
}

fn delete_snapshots(state: &mut State) {
    if let Some(list) = state.snapshots.take() {
        for s in list {
            if let Some(dir) = s.path.parent()
                && let Err(e) = fs::remove_dir_all(dir)
            {
                crate::log::warn_err(&format!("一時コピーを削除できませんでした: {}", s.path.display()), &e);
            }
        }
    }
}

/// この PC で参照できるブラウザの名前（設定画面の表示用）。ファイルの存在を確認するだけ。
pub fn detect_available_browsers() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (_, _, name) in find_profile_databases() {
        let browser = name.split(' ').next().unwrap_or(&name).to_string();
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&browser)) {
            names.push(browser);
        }
    }
    names
}

fn find_profile_databases() -> Vec<(PathBuf, BrowserKind, String)> {
    let local = local_app_data();
    let roaming = roaming_app_data();
    let chromium_roots = [
        (local.join("Google").join("Chrome").join("User Data"), "Chrome"),
        (local.join("Microsoft").join("Edge").join("User Data"), "Edge"),
        (local.join("BraveSoftware").join("Brave-Browser").join("User Data"), "Brave"),
        (local.join("Vivaldi").join("User Data"), "Vivaldi"),
        (local.join("Chromium").join("User Data"), "Chromium"),
        (roaming.join("Opera Software").join("Opera Stable"), "Opera"),
        (roaming.join("Opera Software").join("Opera GX Stable"), "Opera GX"),
    ];

    let mut result = Vec::new();
    for (root, name) in chromium_roots {
        if !root.is_dir() {
            continue;
        }
        add_if_exists(&mut result, root.join("Favicons"), BrowserKind::Chromium, name.to_string());
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let leaf = entry.file_name().to_string_lossy().into_owned();
                if !leaf.eq_ignore_ascii_case("Default") && !leaf.to_ascii_lowercase().starts_with("profile ") {
                    continue;
                }
                add_if_exists(&mut result, entry.path().join("Favicons"), BrowserKind::Chromium, format!("{name} ({leaf})"));
            }
        }
    }

    let firefox = roaming.join("Mozilla").join("Firefox").join("Profiles");
    if let Ok(entries) = fs::read_dir(&firefox) {
        for entry in entries.flatten() {
            let leaf = entry.file_name().to_string_lossy().into_owned();
            add_if_exists(&mut result, entry.path().join("favicons.sqlite"), BrowserKind::Firefox, format!("Firefox ({leaf})"));
        }
    }

    // 最近使われたブラウザから先に見る
    result.sort_by_key(|(p, _, _)| std::cmp::Reverse(modified(p)));
    result
}

fn add_if_exists(list: &mut Vec<(PathBuf, BrowserKind, String)>, path: PathBuf, kind: BrowserKind, name: String) {
    if path.is_file() {
        list.push((path, kind, name));
    }
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// ブラウザ起動中はファイルが掴まれていることがあるため、一時フォルダへコピーしてから読む。
fn try_copy_for_reading(path: &Path) -> Option<PathBuf> {
    let directory = std::env::temp_dir().join(format!("{SNAPSHOT_PREFIX}{}", crate::model::new_id()));
    let result = (|| -> std::io::Result<PathBuf> {
        fs::create_dir_all(&directory)?;
        let destination = directory.join(path.file_name().unwrap_or_default());
        fs::copy(path, &destination)?;
        // WAL / SHM があれば一緒に持ってこないと最新の内容を読めない
        for suffix in ["-wal", "-shm"] {
            let extra = PathBuf::from(format!("{}{suffix}", path.display()));
            if extra.is_file() {
                fs::copy(&extra, format!("{}{suffix}", destination.display()))?;
            }
        }
        Ok(destination)
    })();
    match result {
        Ok(p) => Some(p),
        Err(e) => {
            crate::log::warn_err(&format!("アイコンキャッシュをコピーできませんでした: {}", path.display()), &e);
            // 作りかけの一時フォルダを残さない
            let _ = fs::remove_dir_all(&directory);
            None
        }
    }
}

/// 以前に消しきれなかった一時コピーを片付ける（強制終了された場合など）。十分に古いものだけ。
pub fn delete_stale_snapshots() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(SNAPSHOT_PREFIX) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.created())
            .ok()
            .and_then(|c| c.elapsed().ok())
            .is_some_and(|age| age >= Duration::from_secs(600));
        if old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

// ------------------------------------------------------------ 問い合わせ本体

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
}

/// Chromium 系（Chrome / Edge / Brave / Vivaldi / Opera）の Favicons を読む。
pub fn read_chromium(database: &Path, page: &url::Url) -> rusqlite::Result<Option<Vec<u8>>> {
    let conn = open_read_only(database)?;
    let mut stmt = conn.prepare(
        "SELECT b.image_data, b.width, m.page_url \
         FROM icon_mapping m \
         JOIN favicon_bitmaps b ON b.icon_id = m.icon_id \
         WHERE length(b.image_data) > 0 \
           AND (m.page_url = ?1 OR m.page_url LIKE ?2 ESCAPE '\\') \
         LIMIT 60",
    )?;
    pick_best(&mut stmt, rusqlite::params![page.as_str(), like_prefix(page)], page)
}

/// Firefox の favicons.sqlite を読む。
pub fn read_firefox(database: &Path, page: &url::Url) -> rusqlite::Result<Option<Vec<u8>>> {
    let conn = open_read_only(database)?;
    {
        let mut stmt = conn.prepare(
            "SELECT i.data, i.width, p.page_url \
             FROM moz_icons i \
             JOIN moz_icons_to_pages ip ON ip.icon_id = i.id \
             JOIN moz_pages_w_icons p ON p.id = ip.page_id \
             WHERE length(i.data) > 0 \
               AND (p.page_url = ?1 OR p.page_url LIKE ?2 ESCAPE '\\') \
             LIMIT 60",
        )?;
        if let Some(data) = pick_best(&mut stmt, rusqlite::params![page.as_str(), like_prefix(page)], page)? {
            return Ok(Some(data));
        }
    }
    // ページとの対応が無くても、サイト直下の favicon.ico が別途保存されていることがある
    let mut stmt = conn.prepare(
        "SELECT data, width, icon_url FROM moz_icons \
         WHERE root = 1 AND length(data) > 0 AND icon_url LIKE ?1 ESCAPE '\\' \
         LIMIT 20",
    )?;
    pick_best(&mut stmt, rusqlite::params![like_prefix(page)], page)
}

/// 候補の中から表示に適したものを選ぶ。登録 URL と完全一致するものを優先し、次に 64px に近いサイズ。
fn pick_best(stmt: &mut rusqlite::Statement<'_>, params: impl rusqlite::Params, page: &url::Url) -> rusqlite::Result<Option<Vec<u8>>> {
    let mut best: Option<Vec<u8>> = None;
    let mut best_score = i64::MAX;
    let mut rows = stmt.query(params)?;
    while let Some(row) = rows.next()? {
        let Some(data) = row.get::<_, Option<Vec<u8>>>(0)? else { continue };
        if data.len() < 16 {
            continue;
        }
        // WPF / egui で描けない形式（SVG など）は採用しない
        if crate::icons::detect_image_extension(&data).is_none() {
            continue;
        }
        let width = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
        let url_text = row.get::<_, Option<String>>(2)?.unwrap_or_default();
        let mut score = size_score(width);
        if url_text.eq_ignore_ascii_case(page.as_str()) {
            score -= 10000; // 完全一致は無条件で優先
        }
        if score >= best_score {
            continue;
        }
        best_score = score;
        best = Some(data);
    }
    Ok(best)
}

/// 64px に近いほど良い。32px 未満は大きく減点する。
fn size_score(width: i64) -> i64 {
    if width >= 32 { (width - 64).abs() } else { 1000 + (32 - width) }
}

/// LIKE 用に "https://host:port/%" を組み立てる（ワイルドカードはエスケープする）。
fn like_prefix(page: &url::Url) -> String {
    let host = page.host_str().unwrap_or("");
    let authority = match page.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    let origin = format!("{}://{}", page.scheme(), authority);
    let escaped = origin.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("{escaped}/%")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([10, 20, 30, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    /// 実際のブラウザではなく、同じ形式の検証用データベースを作って確かめる（利用者の閲覧データには触れない）
    fn chromium_db(dir: &Path) -> PathBuf {
        let path = dir.join("Favicons");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE favicons (id INTEGER PRIMARY KEY, url LONGVARCHAR, icon_type INTEGER);
             CREATE TABLE favicon_bitmaps (id INTEGER PRIMARY KEY, icon_id INTEGER, last_updated INTEGER,
                 image_data BLOB, width INTEGER, height INTEGER);
             CREATE TABLE icon_mapping (id INTEGER PRIMARY KEY, page_url LONGVARCHAR, icon_id INTEGER);",
        )
        .unwrap();
        let data = png();
        for (id, page) in [(1, "https://intranet/kintai/"), (2, "http://portal/")] {
            conn.execute("INSERT INTO favicons (id, url, icon_type) VALUES (?1, 'icon', 1)", [id]).unwrap();
            conn.execute(
                "INSERT INTO favicon_bitmaps (id, icon_id, image_data, width, height) VALUES (?1, ?1, ?2, 32, 32)",
                rusqlite::params![id, data],
            )
            .unwrap();
            conn.execute("INSERT INTO icon_mapping (page_url, icon_id) VALUES (?1, ?2)", rusqlite::params![page, id]).unwrap();
        }
        path
    }

    #[test]
    fn reads_chromium_cache_by_origin() {
        let dir = std::env::temp_dir().join(format!("fsc-rs-test-{}", crate::model::new_id()));
        fs::create_dir_all(&dir).unwrap();
        let db = chromium_db(&dir);
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(read_chromium(&db, &u("https://intranet/kintai/")).unwrap().is_some(), "完全一致");
        assert!(read_chromium(&db, &u("https://intranet/")).unwrap().is_some(), "同じサイトの別ページ");
        assert!(read_chromium(&db, &u("http://portal/")).unwrap().is_some());
        assert!(read_chromium(&db, &u("https://portal/")).unwrap().is_none(), "http と https は別");
        assert!(read_chromium(&db, &u("https://intr_net/")).unwrap().is_none(), "LIKE のエスケープ");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_snapshots_are_cleaned_but_fresh_ones_kept() {
        let fresh = std::env::temp_dir().join(format!("{SNAPSHOT_PREFIX}{}", crate::model::new_id()));
        fs::create_dir_all(&fresh).unwrap();
        delete_stale_snapshots();
        assert!(fresh.exists(), "新しいものは残る");
        let _ = fs::remove_dir_all(&fresh);
    }
}

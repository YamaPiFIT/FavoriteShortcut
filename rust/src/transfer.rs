//! エクスポート / インポート / バックアップ（C# 版 ExportImportService・AutoBackupService の移植）。
//!
//! エクスポートは ZIP 1 つにまとめる（C# 版と同じ形式。どちらで作った ZIP もどちらでも読める）:
//!   database.sqlite  … VACUUM INTO で作った整合性のとれたコピー
//!   icons/           … 使われている favicon とカスタムアイコン
//!   settings.json    … 設定（人が読める形）
//!   manifest.json    … アプリ名/バージョン/スキーマ版/件数
//!
//! インポートは必ず先に現在の DB をバックアップしてから行うので、失敗しても元に戻せる（§47）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::db::{CURRENT_SCHEMA_VERSION, Database, local_now_iso};
use crate::loc;
use crate::model::{AppSettings, Folder};
use crate::paths;
use crate::store::Store;
use crate::util::culture_cmp;

const MANIFEST_ENTRY: &str = "manifest.json";
const DATABASE_ENTRY: &str = "database.sqlite";
const SETTINGS_ENTRY: &str = "settings.json";
const ICONS_PREFIX: &str = "icons/";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImportMode {
    /// 現在のデータを残したまま追加する
    Merge,
    /// 現在のデータをすべて置き換える
    Replace,
}

#[derive(Clone, Debug)]
pub struct ImportSummary {
    pub folders: usize,
    pub shortcuts: usize,
    pub skipped: usize,
    pub backup_path: PathBuf,
}

/// manifest.json（C# 版と同じ項目名）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "PascalCase")]
pub struct ExportManifest {
    #[serde(default)]
    pub application: String,
    #[serde(default)]
    pub format_version: i64,
    #[serde(default)]
    pub schema_version: i64,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub exported_at: String,
    #[serde(default)]
    pub folder_count: i64,
    #[serde(default)]
    pub shortcut_count: i64,
    #[serde(default)]
    pub tag_count: i64,
}

type Error = Box<dyn std::error::Error>;

pub fn suggested_file_name() -> String {
    format!("favorite-shortcuts-{}.zip", chrono::Local::now().format("%Y%m%d-%H%M%S"))
}

// -------------------------------------------------------------------- export

pub fn export(store: &Store, settings: &AppSettings, zip_path: &Path) -> Result<(), Error> {
    let temp = std::env::temp_dir().join(format!("fsc-export-{}", crate::model::new_id()));
    fs::create_dir_all(&temp)?;
    let result = (|| -> Result<(), Error> {
        let db_copy = temp.join(DATABASE_ENTRY);
        store.database().vacuum_into(&db_copy)?;

        let manifest = ExportManifest {
            application: "お気に入りショートカット".into(),
            format_version: 1,
            schema_version: CURRENT_SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            exported_at: local_now_iso(),
            folder_count: store.folder_count() as i64,
            shortcut_count: store.shortcut_count() as i64,
            tag_count: store.tag_count() as i64,
        };

        // アイコンは「実際に使われているもの」だけを入れる（ZIP が無駄に膨らまないように）
        let used: HashSet<String> = store.shortcuts().iter().filter_map(|s| s.icon_path.as_deref()).map(str::to_uppercase).collect();

        if zip_path.exists() {
            fs::remove_file(zip_path)?;
        }
        let file = fs::File::create(zip_path)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        zip.start_file(MANIFEST_ENTRY, options)?;
        zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        zip.start_file(SETTINGS_ENTRY, options)?;
        zip.write_all(serde_json::to_string_pretty(&settings_json(settings))?.as_bytes())?;
        zip.start_file(DATABASE_ENTRY, options)?;
        zip.write_all(&fs::read(&db_copy)?)?;

        if let Ok(entries) = fs::read_dir(paths::icon_directory()) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !used.contains(&name.to_uppercase()) {
                    continue;
                }
                zip.start_file(format!("{ICONS_PREFIX}{name}"), options)?;
                zip.write_all(&fs::read(entry.path())?)?;
            }
        }
        zip.finish()?;
        crate::log::info(&format!("エクスポートしました: {}", zip_path.display()));
        Ok(())
    })();
    let _ = fs::remove_dir_all(&temp);
    result
}

/// settings.json の中身（C# 版の AppSettings をそのまま JSON にしたものと同じ形）。
fn settings_json(s: &AppSettings) -> serde_json::Value {
    let mut v = serde_json::json!({
        "ViewMode": s.view_mode as i32,
        "IconSize": s.icon_size,
        "Theme": s.theme as i32,
        "Language": s.language as i32,
        "HotKeyModifiers": s.hotkey_modifiers,
        "HotKeyKey": s.hotkey_key,
        "ClipboardHotKeyModifiers": s.clipboard_hotkey_modifiers,
        "ClipboardHotKeyKey": s.clipboard_hotkey_key,
        "MinimizeToTray": s.minimize_to_tray,
        "StartMinimized": s.start_minimized,
        "RunAtStartup": s.run_at_startup,
        "UseBrowserIconCache": s.use_browser_icon_cache,
        "ShowRecentInLauncher": s.show_recent_in_launcher,
        "LauncherMaxResults": s.launcher_max_results,
        "RestoreLastFolder": s.restore_last_folder,
        "AutoBackup": s.auto_backup,
        "MainWindowWidth": s.main_window_width,
        "MainWindowHeight": s.main_window_height,
        "FolderPaneWidth": s.folder_pane_width,
    });
    if let Some(id) = &s.last_folder_id {
        v["LastFolderId"] = serde_json::Value::String(id.clone());
    }
    v
}

// -------------------------------------------------------------------- import

/// ZIP の中身を読んで、内容の概要を返す（インポート前の確認表示用）。このアプリの ZIP でなければ None。
pub fn peek_manifest(zip_path: &Path) -> Option<ExportManifest> {
    let file = fs::File::open(zip_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(MANIFEST_ENTRY).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

pub fn import(store: &mut Store, zip_path: &Path, mode: ImportMode) -> Result<ImportSummary, Error> {
    let temp = std::env::temp_dir().join(format!("fsc-import-{}", crate::model::new_id()));
    fs::create_dir_all(&temp)?;
    let result = (|| -> Result<ImportSummary, Error> {
        extract_safely(zip_path, &temp)?;

        let imported_path = temp.join(DATABASE_ENTRY);
        if !imported_path.is_file() {
            return Err(loc::t("Str.Transfer.NoDatabase").into());
        }

        // 何かあっても戻せるよう、先に現在の DB を退避する
        let backup_path = create_backup(store, "import-before")?;

        let imported = Store::open(Database::open(&imported_path)?)?;

        store.begin_batch();
        let counts = match mode {
            ImportMode::Replace => import_replace(store, &imported, &temp),
            ImportMode::Merge => import_merge(store, &imported, &temp),
        };
        store.end_batch();
        drop(imported);
        let (folders, shortcuts, skipped) = counts?;
        store.reload()?;

        crate::log::info(&format!("インポート完了 ({mode:?}): フォルダ {folders} / ショートカット {shortcuts} / スキップ {skipped}"));
        Ok(ImportSummary { folders, shortcuts, skipped, backup_path })
    })();
    let _ = fs::remove_dir_all(&temp);
    result
}

fn import_replace(store: &mut Store, imported: &Store, temp: &Path) -> Result<(usize, usize, usize), Error> {
    copy_icons(temp);
    store.clear_all_data(true)?;
    // 件数が多くても速いよう、まとめて 1 つのトランザクションで書き込む
    store.begin_raw_transaction()?;
    let written = (|| -> Result<(), Error> {
        // 親より先に子を入れると外部キー制約に触れるので、親から順に入れる
        for folder in order_by_depth(imported) {
            store.insert_folder_raw(folder)?;
        }
        for shortcut in imported.shortcuts() {
            store.insert_shortcut_raw(shortcut)?;
        }
        Ok(())
    })();
    match written {
        Ok(()) => store.commit_raw_transaction()?,
        Err(e) => {
            store.rollback_raw_transaction();
            return Err(e);
        }
    }
    let raw: Vec<(String, String)> = imported.load_settings_raw()?.into_iter().collect();
    store.save_settings_raw(&raw)?;
    Ok((imported.folder_count(), imported.shortcut_count(), 0))
}

fn import_merge(store: &mut Store, imported: &Store, temp: &Path) -> Result<(usize, usize, usize), Error> {
    copy_icons(temp);

    // 取り込み元のフォルダ ID -> 取り込み先のフォルダ ID
    let mut folder_map: HashMap<String, String> = HashMap::new();
    // 既存フォルダを「フルパス」で引けるようにしておき、同じ構成なら作り直さない
    let mut existing_by_path: HashMap<String, String> = store.folders().map(|f| (f.full_path.to_lowercase(), f.id.clone())).collect();

    let mut added_folders = 0;
    for folder in order_by_depth(imported) {
        let path = build_path(imported, folder);
        if let Some(existing) = existing_by_path.get(&path.to_lowercase()) {
            folder_map.insert(folder.id.clone(), existing.clone());
            continue;
        }
        let parent = folder.parent_id.as_ref().and_then(|p| folder_map.get(p)).cloned();
        let created = store.create_folder(&folder.name, parent.as_deref())?;
        let created_path = store.folder(&created).map(|f| f.full_path.clone()).unwrap_or_default();
        folder_map.insert(folder.id.clone(), created.clone());
        existing_by_path.insert(created_path.to_lowercase(), created);
        added_folders += 1;
    }

    // 既存と同じ内容（フォルダ・タイトル・対象が一致）のショートカットは追加しない
    let mut existing_keys: HashSet<String> = store
        .shortcuts()
        .iter()
        .map(|s| {
            let folder_path = s.folder_id.as_deref().and_then(|f| store.folder(f)).map(|f| f.full_path.clone()).unwrap_or_default();
            duplicate_key(&folder_path, &s.title, &s.target)
        })
        .collect();

    let mut added = 0;
    let mut skipped = 0;
    for source in imported.shortcuts() {
        let folder_path = source.folder_id.as_deref().and_then(|f| imported.folder(f)).map(|f| build_path(imported, f)).unwrap_or_default();
        if !existing_keys.insert(duplicate_key(&folder_path, &source.title, &source.target)) {
            skipped += 1;
            continue;
        }
        let mut copy = source.clone();
        copy.id = crate::model::new_id();
        copy.folder_id = source.folder_id.as_ref().and_then(|f| folder_map.get(f)).cloned();
        let usage = (source.usage_count, source.last_used_at);
        let id = store.add_shortcut(copy)?;
        // 使用履歴も引き継ぐ（add_shortcut は履歴を書かないのでここで反映）
        if (usage.0 > 0 || usage.1.is_some())
            && let Some(item) = store.find_shortcut(&id).cloned()
        {
            let mut item = item;
            item.usage_count = usage.0;
            item.last_used_at = usage.1;
            store.insert_shortcut_raw(&item)?;
        }
        added += 1;
    }
    Ok((added_folders, added, skipped))
}

fn duplicate_key(folder_path: &str, title: &str, target: &str) -> String {
    format!("{folder_path}{title}{target}").to_uppercase()
}

/// 親フォルダが必ず先に来る順序で列挙する。
fn order_by_depth(store: &Store) -> Vec<&Folder> {
    let depth = |f: &Folder| {
        let mut d = 0;
        let mut current = f.parent_id.as_deref().and_then(|p| store.folder(p));
        while let Some(parent) = current {
            if d >= 1000 {
                break;
            }
            d += 1;
            current = parent.parent_id.as_deref().and_then(|p| store.folder(p));
        }
        d
    };
    let mut list: Vec<&Folder> = store.folders().collect();
    list.sort_by(|a, b| depth(a).cmp(&depth(b)).then(a.sort_order.cmp(&b.sort_order)).then_with(|| culture_cmp(&a.name, &b.name)));
    list
}

fn build_path(store: &Store, folder: &Folder) -> String {
    let mut parts = vec![folder.name.as_str()];
    let mut current = folder.parent_id.as_deref().and_then(|p| store.folder(p));
    let mut guard = 0;
    while let Some(f) = current {
        if guard > 1000 {
            break;
        }
        guard += 1;
        parts.push(&f.name);
        current = f.parent_id.as_deref().and_then(|p| store.folder(p));
    }
    parts.reverse();
    parts.join(" > ")
}

fn copy_icons(temp: &Path) {
    let source = temp.join("icons");
    let Ok(entries) = fs::read_dir(&source) else { return };
    let _ = fs::create_dir_all(paths::icon_directory());
    for entry in entries.flatten() {
        let dest = paths::icon_directory().join(entry.file_name());
        if let Err(e) = fs::copy(entry.path(), &dest) {
            crate::log::warn_err(&format!("アイコンのコピーに失敗: {}", entry.path().display()), &e);
        }
    }
}

/// ZIP を展開する。展開先から外に出るパス（Zip Slip）を持つエントリや、想定外のファイルは無視する。
fn extract_safely(zip_path: &Path, destination: &Path) -> Result<(), Error> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name()?.replace('\\', "/");
        if name.ends_with('/') {
            continue;
        }
        // manifest / database / settings / icons 配下だけを受け入れる
        let allowed =
            name == MANIFEST_ENTRY || name == DATABASE_ENTRY || name == SETTINGS_ENTRY || name.to_lowercase().starts_with(ICONS_PREFIX);
        if !allowed {
            continue;
        }
        // 外に出るパス（..、絶対パス、ドライブ指定）は受け付けない
        let Some(relative) = entry.enclosed_name() else { continue };
        if relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            continue;
        }
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

// -------------------------------------------------------------------- backup

/// 現在の DB を backup/ にコピーする。戻り値は作成したファイルのパス。
pub fn create_backup(store: &Store, reason: &str) -> Result<PathBuf, Error> {
    fs::create_dir_all(paths::backup_directory())?;
    let name = format!("{}-{reason}.sqlite", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let path = paths::backup_directory().join(name);
    store.database().vacuum_into(&path)?;
    prune_backups(10);
    Ok(path)
}

fn prune_backups(keep: usize) {
    let Ok(entries) = fs::read_dir(paths::backup_directory()) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sqlite")) && !is_auto_backup(p))
        .map(|p| (fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(keep) {
        if let Err(e) = fs::remove_file(&path) {
            crate::log::warn_err("古いバックアップの整理に失敗しました。", &e);
        }
    }
}

// --------------------------------------------------------------- 自動バックアップ

/// 自動バックアップを残す数（C# 版と同じ）。
pub const AUTO_KEEP_COUNT: usize = 14;
const AUTO_SUFFIX: &str = "-auto.sqlite";
const STATE_FILE_NAME: &str = "auto-backup.json";

pub fn is_auto_backup(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().ends_with(AUTO_SUFFIX))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BackupState {
    created_at: String,
    fingerprint: String,
}

fn state_path() -> PathBuf {
    paths::backup_directory().join(STATE_FILE_NAME)
}

fn load_state() -> Option<BackupState> {
    let text = fs::read_to_string(state_path()).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// 最後に自動バックアップを作った日時（設定画面の表示用）。
pub fn last_auto_backup_time() -> Option<chrono::DateTime<chrono::Local>> {
    let s = load_state()?;
    chrono::DateTime::parse_from_rfc3339(&s.created_at).ok().map(|d| d.with_timezone(&chrono::Local))
}

/// 1 日 1 回の自動バックアップ（登録内容が変わった日だけ）。
/// 起動して 1 分後と、起動したままなら 30 分ごとに確かめる。コピーは裏のスレッドで行う。
pub struct AutoBackup {
    next_check: Instant,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Default for AutoBackup {
    fn default() -> Self {
        Self {
            next_check: Instant::now() + std::time::Duration::from_secs(60),
            running: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl AutoBackup {
    /// 次に確かめる時刻（画面のスレッドの待ち時間の計算に使う）。
    pub fn next_check(&self) -> Instant {
        self.next_check
    }

    pub fn tick(&mut self, store: &Store, settings: &AppSettings) {
        if Instant::now() < self.next_check {
            return;
        }
        self.next_check = Instant::now() + std::time::Duration::from_secs(30 * 60);
        if self.running.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let Some(fingerprint) = due_fingerprint(store, settings) else { return };
        let db_path = store.database().file_path().to_path_buf();
        let running = self.running.clone();
        running.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = std::thread::Builder::new().name("AutoBackup".into()).spawn(move || {
            match create_auto_backup(&db_path, &fingerprint) {
                Ok(path) => crate::log::info(&format!("自動バックアップを作成しました: {}", path.display())),
                Err(e) => crate::log::warn_err("自動バックアップに失敗しました。", &e),
            }
            running.store(false, std::sync::atomic::Ordering::Relaxed);
        });
    }
}

/// 今バックアップを作るべきなら、そのときの登録内容の指紋を返す。
fn due_fingerprint(store: &Store, settings: &AppSettings) -> Option<String> {
    if !settings.auto_backup {
        return None;
    }
    let state = load_state();
    if let Some(s) = &state
        && let Ok(created) = chrono::DateTime::parse_from_rfc3339(&s.created_at)
        && created.with_timezone(&chrono::Local).date_naive() == chrono::Local::now().date_naive()
    {
        return None; // 今日は作成済み
    }
    let fingerprint = compute_fingerprint(store)?;
    if state.is_some_and(|s| s.fingerprint == fingerprint) {
        return None;
    }
    Some(fingerprint)
}

/// 登録内容の指紋（C# 版と同じ式）。件数と最終更新日時から作るので、追加・変更・削除のどれでも変わる。
fn compute_fingerprint(store: &Store) -> Option<String> {
    store
        .database()
        .conn()
        .query_row(
            "SELECT (SELECT count(*) FROM folders) || '|' || ifnull((SELECT max(updated_at) FROM folders), '') \
                 || '|' || (SELECT count(*) FROM shortcuts) || '|' || ifnull((SELECT max(updated_at) FROM shortcuts), '') \
                 || '|' || (SELECT count(*) FROM tags) || '|' || (SELECT count(*) FROM shortcut_tags)",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
}

/// 画面が使っている接続とは別の接続でコピーする（WAL なので、ある時点の整合性のとれた内容になる）。
fn create_auto_backup(db_path: &Path, fingerprint: &str) -> Result<PathBuf, Error> {
    fs::create_dir_all(paths::backup_directory())?;
    let name = format!("{}{AUTO_SUFFIX}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let path = paths::backup_directory().join(name);
    let temp = PathBuf::from(format!("{}.tmp", path.display()));
    if temp.exists() {
        fs::remove_file(&temp)?;
    }
    {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute("VACUUM INTO ?1", [temp.to_string_lossy().as_ref()])?;
    }
    fs::rename(&temp, &path)?;
    let state = BackupState { created_at: local_now_iso(), fingerprint: fingerprint.to_string() };
    fs::write(state_path(), serde_json::to_string(&state)?)?;
    prune_auto();
    Ok(path)
}

fn prune_auto() {
    let Ok(entries) = fs::read_dir(paths::backup_directory()) else { return };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| is_auto_backup(p)).collect();
    files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    for path in files.into_iter().skip(AUTO_KEEP_COUNT) {
        if let Err(e) = fs::remove_file(&path) {
            crate::log::warn_err(&format!("古い自動バックアップを削除できませんでした: {}", path.display()), &e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_uses_csharp_property_names() {
        let m = ExportManifest { application: "x".into(), shortcut_count: 3, ..Default::default() };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"ShortcutCount\":3"));
        let back: ExportManifest = serde_json::from_str("{\"Application\":\"a\",\"FolderCount\":2}").unwrap();
        assert_eq!(back.folder_count, 2);
    }

    #[test]
    fn auto_backups_are_recognized_by_name() {
        assert!(is_auto_backup(Path::new("C:\\x\\20261001-090000-auto.sqlite")));
        assert!(!is_auto_backup(Path::new("C:\\x\\20261001-090000-manual.sqlite")));
    }
}

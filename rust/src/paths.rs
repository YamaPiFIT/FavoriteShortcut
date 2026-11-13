//! アプリが使用するフォルダ / ファイルの場所（C# 版 AppPaths と同じ規則）。
//!
//! 既定は「EXE と同じ場所の Data フォルダ」。C# 版の EXE と同じフォルダに置けば、
//! 同じ Data（同じデータベース）を使う。
//!
//! 解決の優先順位:
//!   1. 環境変数 FAVORITESHORTCUT_DATA_DIR
//!   2. EXE と同じ場所の location.json（設定画面で変更したとき）
//!   3. %APPDATA% の location.json（EXE 側に書けない環境で変更したとき）
//!   4. EXE と同じ場所の Data フォルダ  ← 既定
//!   5. 4 に書き込めない場合のみ %APPDATA%\お気に入りショートカット\

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const APP_FOLDER_NAME: &str = "お気に入りショートカット";
pub const DATA_DIRECTORY_ENV: &str = "FAVORITESHORTCUT_DATA_DIR";
const LOCATION_CONFIG_FILE_NAME: &str = "location.json";

struct Resolved {
    data_directory: PathBuf,
    using_fallback: bool,
}

static RESOLVED: OnceLock<Resolved> = OnceLock::new();

pub fn exe_directory() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

/// 既定のデータ保存場所（EXE と同じ場所の Data）。
pub fn default_data_directory() -> PathBuf {
    exe_directory().join("Data")
}

/// EXE の場所に書き込めない場合の退避先。
pub fn fallback_data_directory() -> PathBuf {
    roaming_app_data().join(APP_FOLDER_NAME)
}

fn exe_location_config_file() -> PathBuf {
    exe_directory().join(LOCATION_CONFIG_FILE_NAME)
}

fn fallback_location_config_file() -> PathBuf {
    fallback_data_directory().join(LOCATION_CONFIG_FILE_NAME)
}

pub fn roaming_app_data() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// 環境変数で保存場所を指定しているか（検証用。C# 版と同時に動かせるようにするため）。
pub fn is_overridden_by_environment() -> bool {
    std::env::var_os(DATA_DIRECTORY_ENV).is_some_and(|v| !v.is_empty())
}

fn resolved() -> &'static Resolved {
    RESOLVED.get_or_init(|| {
        let (dir, using_fallback) = resolve_data_directory();
        let _ = fs::create_dir_all(&dir);
        let _ = fs::create_dir_all(dir.join("icons"));
        let _ = fs::create_dir_all(dir.join("backup"));
        Resolved { data_directory: dir, using_fallback }
    })
}

/// DB・アイコン・ログを格納する実際のフォルダ。
pub fn data_directory() -> PathBuf {
    resolved().data_directory.clone()
}

/// EXE の場所に書き込めず、%APPDATA% へ退避した場合 true。
pub fn using_fallback_location() -> bool {
    resolved().using_fallback
}

pub fn database_file() -> PathBuf {
    data_directory().join("favorite-shortcuts.sqlite")
}

pub fn icon_directory() -> PathBuf {
    data_directory().join("icons")
}

pub fn backup_directory() -> PathBuf {
    data_directory().join("backup")
}

pub fn log_file() -> PathBuf {
    data_directory().join("app.log")
}

fn resolve_data_directory() -> (PathBuf, bool) {
    if let Some(v) = std::env::var_os(DATA_DIRECTORY_ENV)
        && !v.is_empty()
    {
        return (absolute(Path::new(&v)), false);
    }

    // 設定画面で保存場所を変更した場合。EXE 隣の設定を優先する
    for config in [exe_location_config_file(), fallback_location_config_file()] {
        if let Some(dir) = try_read_location_config(&config) {
            return (dir, false);
        }
    }

    let default = default_data_directory();
    if can_write_to(&default) {
        return (default, false);
    }

    (fallback_data_directory(), true)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LocationConfig {
    #[serde(rename = "DataDirectory")]
    data_directory: Option<String>,
}

fn try_read_location_config(path: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(path).ok()?;
    // C# 版は BOM 付きで書くことがあるので取り除く
    let text = text.trim_start_matches('\u{feff}');
    let cfg: LocationConfig = serde_json::from_str(text).ok()?;
    let dir = cfg.data_directory?;
    if dir.trim().is_empty() {
        return None;
    }
    Some(absolute(Path::new(&dir)))
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 実際にファイルを作って、その場所に書き込めるかどうかを確かめる。
fn can_write_to(directory: &Path) -> bool {
    if fs::create_dir_all(directory).is_err() {
        return false;
    }
    let probe = directory.join(format!(".write-test-{}", uuid::Uuid::new_v4().simple()));
    if fs::write(&probe, b"").is_err() {
        return false;
    }
    let _ = fs::remove_file(&probe);
    true
}

/// データ保存場所を変更する（次回起動から有効）。None を渡すと既定（EXE 隣の Data）に戻る。
pub fn save_data_directory_override(directory: Option<&Path>) -> std::io::Result<()> {
    // 開発・確認用に保存場所を環境変数で指定しているときは、普段使いの設定ファイルに触れない
    if is_overridden_by_environment() {
        return Err(std::io::Error::other(format!("{DATA_DIRECTORY_ENV} で保存場所を指定して起動しているため、変更できません。")));
    }
    let exe_writable = can_write_to(&exe_directory());
    let target = if exe_writable { exe_location_config_file() } else { fallback_location_config_file() };
    if !exe_writable {
        fs::create_dir_all(fallback_data_directory())?;
    }

    let Some(directory) = directory else {
        delete_if_exists(&exe_location_config_file());
        delete_if_exists(&fallback_location_config_file());
        return Ok(());
    };

    let cfg = LocationConfig { data_directory: Some(directory.to_string_lossy().into_owned()) };
    let json = serde_json::to_string_pretty(&cfg).map_err(std::io::Error::other)?;
    fs::write(&target, json)?;

    // 片方だけが残って解決順序で食い違わないようにする
    if target == exe_location_config_file() {
        delete_if_exists(&fallback_location_config_file());
    }
    Ok(())
}

fn delete_if_exists(path: &Path) {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

//! SQLite 接続とスキーマ管理（C# 版 Database と同じスキーマ・同じ設定）。
//!
//! スキーマのバージョンは PRAGMA user_version で管理する。C# 版と同じ DB を読み書きするため、
//! スキーマを変えるときは必ず C# 版と合わせて変えること（バージョンを片方だけ上げると、
//! もう片方は「新しすぎる DB」として開けなくなる）。

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{Connection, OpenFlags};

use crate::loc;

/// 現在のスキーマバージョン（C# 版 Database.CurrentSchemaVersion と同じ値）。
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

pub struct Database {
    conn: Connection,
    path: PathBuf,
}

#[derive(Debug)]
pub enum DbError {
    Sqlite(rusqlite::Error),
    TooNew(i64),
    Io(std::io::Error),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Sqlite(e) => write!(f, "{e}"),
            DbError::TooNew(v) => {
                write!(f, "{}", loc::tf("Str.Database.TooNew", &[v, &CURRENT_SCHEMA_VERSION]))
            }
            DbError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e)
    }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::Io(e)
    }
}

pub type DbResult<T> = Result<T, DbError>;

impl Database {
    pub fn open(path: &Path) -> DbResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }

        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        // WAL + synchronous=NORMAL により、強制終了時でも DB が壊れにくい構成にする
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;

        let db = Self { conn, path: path.to_path_buf() };
        db.migrate()?;
        Ok(db)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn file_path(&self) -> &Path {
        &self.path
    }

    pub fn schema_version(&self) -> DbResult<i64> {
        Ok(self.conn.query_row("PRAGMA user_version;", [], |r| r.get(0))?)
    }

    fn migrate(&self) -> DbResult<()> {
        let mut version = self.schema_version()?;

        if version > CURRENT_SCHEMA_VERSION {
            return Err(DbError::TooNew(version));
        }

        if version < 1 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(SCHEMA_V1)?;
            tx.commit()?;
            self.conn.pragma_update(None, "user_version", 1)?;
            version = 1;
        }

        // 今後のマイグレーションはここに if version < 2 { ... } の形で追加する（C# 版と同時に）。

        if version != CURRENT_SCHEMA_VERSION {
            self.conn.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        }
        Ok(())
    }

    /// WAL を本体に取り込む（バックアップ / エクスポートの前と、閉じるとき）。
    pub fn checkpoint(&self) {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    /// VACUUM INTO で整合性のとれたコピーを作る。
    pub fn vacuum_into(&self, destination: &Path) -> DbResult<()> {
        self.checkpoint();
        if destination.exists() {
            std::fs::remove_file(destination)?;
        }
        self.conn.execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])?;
        Ok(())
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        self.checkpoint();
    }
}

/// C# 版のスキーマ（Database.cs の SchemaV1）と一字一句同じ。
pub const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS folders (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    parent_id   TEXT NULL REFERENCES folders(id) ON DELETE CASCADE,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_folders_parent ON folders(parent_id, sort_order);

CREATE TABLE IF NOT EXISTS shortcuts (
    id          TEXT PRIMARY KEY,
    folder_id   TEXT NULL REFERENCES folders(id) ON DELETE SET NULL,
    title       TEXT NOT NULL,
    target      TEXT NOT NULL,
    target_type INTEGER NOT NULL DEFAULT 0,
    icon_path   TEXT NULL,
    note        TEXT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_shortcuts_folder ON shortcuts(folder_id, sort_order);
CREATE INDEX IF NOT EXISTS ix_shortcuts_title  ON shortcuts(title);
CREATE INDEX IF NOT EXISTS ix_shortcuts_target ON shortcuts(target);

CREATE TABLE IF NOT EXISTS tags (
    id   TEXT PRIMARY KEY,
    name TEXT NOT NULL COLLATE NOCASE UNIQUE
);
CREATE INDEX IF NOT EXISTS ix_tags_name ON tags(name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS shortcut_tags (
    shortcut_id TEXT NOT NULL REFERENCES shortcuts(id) ON DELETE CASCADE,
    tag_id      TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (shortcut_id, tag_id)
);
CREATE INDEX IF NOT EXISTS ix_shortcut_tags_tag ON shortcut_tags(tag_id);

CREATE TABLE IF NOT EXISTS shortcut_usage (
    shortcut_id  TEXT PRIMARY KEY REFERENCES shortcuts(id) ON DELETE CASCADE,
    last_used_at TEXT NULL,
    usage_count  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ix_usage_last ON shortcut_usage(last_used_at DESC);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NULL
);
";

/// C# 版の DateTime.ToUniversalTime().ToString("o") と同じ形式
/// （例 "2026-11-12T01:02:03.1234567Z"、秒以下 7 桁）。
pub fn format_time(value: DateTime<Utc>) -> String {
    let ticks = value.timestamp_subsec_nanos() / 100; // 100ns 単位 = 7 桁
    format!("{}.{:07}Z", value.format("%Y-%m-%dT%H:%M:%S"), ticks)
}

/// 保存した ISO 8601 文字列を UTC に戻す。解釈できない値でも例外にせず現在時刻にする
/// （1 レコードの不整合でアプリ全体が起動しなくなることを避ける。C# 版と同じ方針）。
pub fn parse_time(value: &str) -> DateTime<Utc> {
    try_parse_time(value).unwrap_or_else(Utc::now)
}

pub fn try_parse_time(value: &str) -> Option<DateTime<Utc>> {
    let v = value.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(v) {
        return Some(dt.with_timezone(&Utc));
    }
    // オフセットの無い値は UTC とみなす（C# 版の ParseTime と同じ）
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(v, fmt) {
            return Some(n.and_utc());
        }
    }
    None
}

/// 現在時刻をローカル時刻の ISO 8601（オフセット付き、C# の DateTime.Now.ToString("o") 相当）で返す。
pub fn local_now_iso() -> String {
    let now = chrono::Local::now();
    let ticks = now.timestamp_subsec_nanos() / 100;
    format!("{}.{:07}{}", now.format("%Y-%m-%dT%H:%M:%S"), ticks, now.format("%:z"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn time_round_trips_in_csharp_format() {
        let t = Utc.with_ymd_and_hms(2026, 11, 12, 1, 2, 3).unwrap() + chrono::Duration::nanoseconds(123_456_700);
        let s = format_time(t);
        assert_eq!(s, "2026-11-12T01:02:03.1234567Z");
        assert_eq!(parse_time(&s), t);
    }

    #[test]
    fn parses_values_written_by_csharp() {
        // C# 版が書いた値（秒以下 7 桁、Z 付き）とオフセット付き・無しの値
        assert!(try_parse_time("2026-10-06T08:09:16.1234567Z").is_some());
        assert!(try_parse_time("2026-10-06T17:09:16.1234567+09:00").is_some());
        assert!(try_parse_time("2026-10-06T08:09:16").is_some());
        assert!(try_parse_time("壊れた値").is_none());
    }

    #[test]
    fn creates_schema_version_1() {
        let dir = std::env::temp_dir().join(format!("fsc-rs-test-{}", crate::model::new_id()));
        let db = Database::open(&dir.join("a.sqlite")).unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
        let tables: i64 = db.conn().query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 6);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_newer_schema() {
        let dir = std::env::temp_dir().join(format!("fsc-rs-test-{}", crate::model::new_id()));
        let path = dir.join("a.sqlite");
        {
            let db = Database::open(&path).unwrap();
            db.conn().pragma_update(None, "user_version", 99).unwrap();
        }
        assert!(matches!(Database::open(&path), Err(DbError::TooNew(99))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

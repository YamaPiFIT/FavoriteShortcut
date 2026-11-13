//! 最低限のファイルログ（C# 版 AppLog と同じ書式・同じファイル）。
//! ログ出力自体が失敗してもアプリを巻き込まないよう、すべて握りつぶす。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

static GATE: Mutex<()> = Mutex::new(());
const MAX_BYTES: u64 = 1024 * 1024; // 1MB を超えたら 1 世代だけ退避

pub fn info(message: &str) {
    write("INFO", message, None);
}

pub fn warn(message: &str) {
    write("WARN", message, None);
}

pub fn warn_err(message: &str, err: &dyn std::fmt::Display) {
    write("WARN", message, Some(&err.to_string()));
}

pub fn error(message: &str) {
    write("ERROR", message, None);
}

pub fn error_err(message: &str, err: &dyn std::fmt::Display) {
    write("ERROR", message, Some(&err.to_string()));
}

fn write(level: &str, message: &str, detail: Option<&str>) {
    let Ok(_guard) = GATE.lock() else { return };
    let path = crate::paths::log_file();
    rotate(&path);

    let mut line = format!("{} [{}] {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), level, message);
    if let Some(d) = detail {
        line.push_str("\r\n");
        line.push_str(d);
    }
    line.push_str("\r\n");

    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

fn rotate(path: &std::path::Path) {
    let Ok(meta) = fs::metadata(path) else { return };
    if meta.len() < MAX_BYTES {
        return;
    }
    let old = path.with_extension("log.1");
    let _ = fs::remove_file(&old);
    let _ = fs::rename(path, &old);
}

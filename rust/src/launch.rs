//! ショートカットの起動（C# 版 LaunchService と同じ動き）。
//! Web は既定ブラウザ、フォルダはエクスプローラー、ファイルは既定のアプリで開く。
//!
//! 開く処理はすべて、画面とは別の専用スレッド（STA）で行う。Windows は開く処理（ShellExecute）の
//! 中でセキュリティの確認（ゾーン判定・署名の失効確認・共有フォルダへの問い合わせ）を行い、
//! ネットワークの状態が悪いと待たされることがあるため（C# 版 v1.3.2 と同じ対策）。

use std::path::Path;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

use crate::loc;
use crate::model::TargetType;
use crate::target;
use crate::util::wide;

#[derive(Clone, Debug)]
pub struct LaunchResult {
    pub success: bool,
    pub error_title: String,
    pub error_detail: String,
}

impl LaunchResult {
    fn ok() -> Self {
        Self { success: true, error_title: String::new(), error_detail: String::new() }
    }

    fn fail(title_key: &str, detail: String) -> Self {
        Self { success: false, error_title: loc::t(title_key), error_detail: detail }
    }
}

/// ショートカットを裏のスレッドで開く。終わったら on_done が（そのスレッドで）呼ばれる。
pub fn launch_async(target: String, kind: TargetType, on_done: impl FnOnce(LaunchResult) + Send + 'static) {
    run_on_sta_thread(move || on_done(launch(&target, kind)));
}

/// エクスプローラーで対象の場所を、裏のスレッドで開く（ファイルなら選択状態にする）。
pub fn reveal_async(target: String, kind: TargetType, on_done: impl FnOnce(LaunchResult) + Send + 'static) {
    run_on_sta_thread(move || on_done(reveal_in_explorer(&target, kind)));
}

/// フォルダやファイルを、裏のスレッドで開く（失敗してもログに残すだけ）。
pub fn open_path(path: impl Into<String>) {
    let path = path.into();
    run_on_sta_thread(move || {
        if let Err(e) = shell_open(&path, None, None) {
            crate::log::warn_err(&format!("パスを開けませんでした: {path}"), &e);
        }
    });
}

/// 1 件ごとにスレッドを分けるので、応答の遅い共有フォルダを開いている間も、次に開くものは待たされない。
pub fn run_on_sta_thread(work: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name("LaunchService".into()).spawn(move || {
        // ShellExecute は COM を使うので STA で呼ぶ
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        work();
        if initialized {
            unsafe { CoUninitialize() };
        }
    });
}

fn launch(target_text: &str, item_kind: TargetType) -> LaunchResult {
    let target = target_text.trim();
    if target.is_empty() {
        return LaunchResult::fail("Str.Launch.CannotOpen", loc::t("Str.Launch.NoTarget"));
    }

    let kind = if item_kind == TargetType::Unknown { target::detect(target) } else { item_kind };
    let expanded = target::expand(target);

    let result = match kind {
        TargetType::Folder => {
            if !Path::new(&expanded).is_dir() {
                return LaunchResult::fail("Str.Launch.FolderNotFound", expanded);
            }
            shell_open(&expanded, None, None)
        }
        TargetType::File | TargetType::Application => {
            if !Path::new(&expanded).is_file() {
                return LaunchResult::fail("Str.Launch.FileNotFound", expanded);
            }
            let working = target::parent_dir(&expanded).map(str::to_string);
            shell_open(&expanded, None, working.as_deref())
        }
        TargetType::Web => shell_open(&target::normalize(&expanded, TargetType::Web), None, None),
        // 種別が判定できないものは、そのままシェルに渡す
        TargetType::Unknown => shell_open(&expanded, None, None),
    };

    match result {
        Ok(()) => LaunchResult::ok(),
        Err(e) => {
            crate::log::warn_err(&format!("起動に失敗: {target}"), &e);
            LaunchResult::fail("Str.Launch.CannotOpen", format!("{expanded}\n\n{e}"))
        }
    }
}

fn reveal_in_explorer(target_text: &str, item_kind: TargetType) -> LaunchResult {
    let kind = if item_kind == TargetType::Unknown { target::detect(target_text) } else { item_kind };
    if kind == TargetType::Web {
        return LaunchResult::fail("Str.Launch.NotSupported", loc::t("Str.Launch.WebHasNoFolder"));
    }

    let expanded = target::expand(target_text);
    let result = if Path::new(&expanded).is_dir() {
        shell_open("explorer.exe", Some(&format!("\"{expanded}\"")), None)
    } else if Path::new(&expanded).is_file() {
        shell_open("explorer.exe", Some(&format!("/select,\"{expanded}\"")), None)
    } else {
        // 実体が無くても、親フォルダが残っていればそこを開く
        match target::parent_dir(&expanded).filter(|p| Path::new(p).is_dir()) {
            Some(parent) => shell_open("explorer.exe", Some(&format!("\"{parent}\"")), None),
            None => return LaunchResult::fail("Str.Launch.LocationNotFound", expanded),
        }
    };

    match result {
        Ok(()) => LaunchResult::ok(),
        Err(e) => {
            crate::log::warn_err(&format!("エクスプローラー表示に失敗: {expanded}"), &e);
            LaunchResult::fail("Str.Launch.CannotOpen", format!("{expanded}\n\n{e}"))
        }
    }
}

/// ShellExecuteEx で開く（C# の Process.Start(UseShellExecute = true) と同じ）。
fn shell_open(file: &str, parameters: Option<&str>, working_dir: Option<&str>) -> windows::core::Result<()> {
    let file_w = wide(file);
    let params_w = parameters.map(wide);
    let dir_w = working_dir.filter(|d| Path::new(d).is_dir()).map(wide);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file_w.as_ptr()),
        lpParameters: params_w.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
        lpDirectory: dir_w.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn wait(f: impl FnOnce(Box<dyn FnOnce(LaunchResult) + Send>)) -> LaunchResult {
        let (tx, rx) = mpsc::channel();
        f(Box::new(move |r| {
            let _ = tx.send(r);
        }));
        rx.recv_timeout(std::time::Duration::from_secs(30)).expect("終わらない")
    }

    // 実際には何も開かない（存在しない場所を使うので、Windows に渡す前に止まる）
    fn missing() -> String {
        std::env::temp_dir().join(format!("fsc-rs-no-such-{}", crate::model::new_id())).to_string_lossy().into_owned()
    }

    #[test]
    fn missing_folder_is_not_opened() {
        let r = wait(|done| launch_async(missing(), TargetType::Folder, done));
        assert!(!r.success);
        assert_eq!(r.error_title, loc::t("Str.Launch.FolderNotFound"));
    }

    #[test]
    fn missing_file_is_not_opened() {
        let path = format!("{}\\memo.txt", missing());
        let r = wait(|done| launch_async(path, TargetType::File, done));
        assert_eq!(r.error_title, loc::t("Str.Launch.FileNotFound"));
    }

    #[test]
    fn reveal_reports_missing_location_and_web() {
        let path = format!("{}\\memo.txt", missing());
        let r = wait(|done| reveal_async(path, TargetType::File, done));
        assert_eq!(r.error_title, loc::t("Str.Launch.LocationNotFound"));
        let r = wait(|done| reveal_async("https://example.com/".into(), TargetType::Web, done));
        assert_eq!(r.error_title, loc::t("Str.Launch.NotSupported"));
    }

    #[test]
    fn work_runs_on_another_sta_thread() {
        let (tx, rx) = mpsc::channel();
        let caller = std::thread::current().id();
        run_on_sta_thread(move || {
            let _ = tx.send(std::thread::current().id());
        });
        let worker = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_ne!(worker, caller);
    }
}

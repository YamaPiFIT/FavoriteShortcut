//! お気に入りショートカット（Rust 版）。
//!
//! C# 版と同じデータ（SQLite のデータベース・アイコン・バックアップ）をそのまま使う。
//! C# 版と同時には動かない（二重起動の防止を共有している）。

#![windows_subsystem = "windows"]

mod app;
mod bookmarks;
mod db;
mod dragdrop;
mod favicon_cache;
mod icons;
mod keys;
mod launch;
mod loc;
mod log;
mod model;
mod normalize;
mod paths;
mod platform;
mod search;
mod settings;
mod shell_icons;
mod store;
mod strings_table;
mod target;
mod transfer;
mod tray;
mod ui;
mod util;

use winit::event_loop::EventLoop;

use crate::model::AppLanguage;
use crate::platform::MsgIcon;

fn main() {
    // 二重起動を防ぐ（C# 版と共通）。すでに動いていれば、そちらのウィンドウを前に出して終了する
    let Some(_instance) = platform::SingleInstance::acquire() else {
        platform::broadcast_show_request();
        return;
    };

    std::panic::set_hook(Box::new(|info| {
        log::error(&format!("処理されない例外: {info}"));
    }));

    // 言語の設定は DB に入っているので、読み込むまでは Windows の表示言語で表示する
    // （DB を開けなかったときのエラーも、利用者が読める言語で出すため）
    loc::apply(AppLanguage::Auto);

    let mut store = match db::Database::open(&paths::database_file()).and_then(store::Store::open) {
        Ok(store) => store,
        Err(e) => {
            log::error_err("起動処理に失敗しました。", &e);
            let text = loc::tf("Str.App.DbOpenFailed", &[&paths::data_directory().display(), &e]);
            platform::message(None, &text, &loc::t("Str.App.Name"), MsgIcon::Error);
            return;
        }
    };
    let settings = settings::load(&store);
    if loc::apply(settings.language) {
        // 「未分類」などの表示を設定の言語で作り直す
        store.refresh_localized_texts();
    }

    // 初回起動時に、使い方が分かる最小限のフォルダを用意する（C# 版と同じ）
    if store.folder_count() == 0 && store.shortcut_count() == 0 {
        for key in ["Str.Folder.SeedWork", "Str.Folder.SeedDevelopment", "Str.Folder.SeedPersonal", "Str.Folder.SeedOther"] {
            if let Err(e) = store.create_folder(&loc::t(key), None) {
                log::warn_err("初期フォルダの作成に失敗しました。", &e);
                break;
            }
        }
    }

    let start_hidden = std::env::args().skip(1).any(|a| a.eq_ignore_ascii_case(platform::TRAY_ARGUMENT)) || settings.start_minimized;

    let event_loop = match EventLoop::<app::UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            app::fatal(&format!("画面の準備に失敗しました: {e}"));
            return;
        }
    };
    let proxy = event_loop.create_proxy();
    let mut application = app::App::new(store, settings, proxy, start_hidden);
    if let Err(e) = event_loop.run_app(&mut application) {
        log::error(&format!("イベントループが異常終了しました: {e}"));
    }
}

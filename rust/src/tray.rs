//! タスクトレイのアイコンとメニュー（C# 版と同じ項目・同じ並び）。
//! メニューの先頭には「最近使った項目」が 10 件並ぶ。

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::loc;

const RECENT_COUNT: usize = 10;
const TITLE_MAX_LENGTH: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayAction {
    OpenMain,
    ShowLauncher,
    AddFromClipboard,
    Settings,
    Exit,
    OpenRecent(String),
}

pub struct Tray {
    icon: TrayIcon,
}

const ID_OPEN_MAIN: &str = "tray.open_main";
const ID_SHOW_LAUNCHER: &str = "tray.show_launcher";
const ID_ADD_CLIPBOARD: &str = "tray.add_clipboard";
const ID_SETTINGS: &str = "tray.settings";
const ID_EXIT: &str = "tray.exit";
const RECENT_PREFIX: &str = "tray.recent:";

impl Tray {
    /// トレイアイコンを作る。メニューの操作やダブルクリックは handler に届く（画面のスレッドで呼ばれる）。
    pub fn create(recent: &[(String, String, String)], handler: impl Fn(TrayAction) + Send + Sync + 'static) -> Option<Self> {
        let handler = std::sync::Arc::new(handler);
        {
            let h = handler.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(action) = action_of(&event.id) {
                    h(action);
                }
            }));
        }
        {
            let h = handler.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::DoubleClick { .. } = event {
                    h(TrayAction::OpenMain);
                }
            }));
        }

        let icon = TrayIconBuilder::new()
            .with_icon(app_icon()?)
            .with_tooltip(loc::t("Str.App.Name"))
            .with_menu(Box::new(build_menu(recent)))
            .with_menu_on_left_click(false)
            .build();
        match icon {
            Ok(icon) => Some(Self { icon }),
            Err(e) => {
                crate::log::warn_err("タスクトレイアイコンを作成できませんでした。", &e);
                None
            }
        }
    }

    /// メニューを作り直す（最近使った項目や表示言語が変わったとき）。
    /// recent は (ID, タイトル, 対象) の並び。
    pub fn rebuild(&self, recent: &[(String, String, String)]) {
        self.icon.set_menu(Some(Box::new(build_menu(recent))));
        let _ = self.icon.set_tooltip(Some(loc::t("Str.App.Name")));
    }
}

fn action_of(id: &MenuId) -> Option<TrayAction> {
    let id = id.0.as_str();
    Some(match id {
        ID_OPEN_MAIN => TrayAction::OpenMain,
        ID_SHOW_LAUNCHER => TrayAction::ShowLauncher,
        ID_ADD_CLIPBOARD => TrayAction::AddFromClipboard,
        ID_SETTINGS => TrayAction::Settings,
        ID_EXIT => TrayAction::Exit,
        _ => TrayAction::OpenRecent(id.strip_prefix(RECENT_PREFIX)?.to_string()),
    })
}

fn build_menu(recent: &[(String, String, String)]) -> Menu {
    let menu = Menu::new();

    let _ = menu.append(&MenuItem::new(loc::t("Str.Launcher.Recent"), false, None));
    if recent.is_empty() {
        let _ = menu.append(&MenuItem::new(loc::t("Str.Tray.NoRecent"), false, None));
    }
    for (id, title, _target) in recent.iter().take(RECENT_COUNT) {
        let mut text = crate::util::truncate_chars(title, TITLE_MAX_LENGTH);
        // & はメニューのアクセスキーの印になるので、文字として表示するよう重ねる
        text = text.replace('&', "&&");
        let _ = menu.append(&MenuItem::with_id(format!("{RECENT_PREFIX}{id}"), text, true, None));
    }
    let _ = menu.append(&PredefinedMenuItem::separator());

    let _ = menu.append(&MenuItem::with_id(ID_OPEN_MAIN, loc::t("Str.Tray.OpenMain"), true, None));
    let _ = menu.append(&MenuItem::with_id(ID_SHOW_LAUNCHER, loc::t("Str.Tray.ShowLauncher"), true, None));
    let _ = menu.append(&MenuItem::with_id(ID_ADD_CLIPBOARD, loc::t("Str.Tray.AddFromClipboard"), true, None));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(ID_SETTINGS, loc::t("Str.Tray.Settings"), true, None));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(ID_EXIT, loc::t("Str.Tray.Exit"), true, None));
    menu
}

/// アプリのアイコン（C# 版と同じ app.ico）。
fn app_icon() -> Option<Icon> {
    let (rgba, w, h) = app_icon_rgba(32)?;
    Icon::from_rgba(rgba, w, h).ok()
}

/// app.ico を指定の大きさの RGBA にする（トレイ・ウィンドウのアイコン用）。
pub fn app_icon_rgba(size: u32) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(include_bytes!("../assets/app.ico")).ok()?;
    let img = img.resize_exact(size, size, image::imageops::FilterType::Lanczos3).to_rgba8();
    let (w, h) = (img.width(), img.height());
    Some((img.into_raw(), w, h))
}

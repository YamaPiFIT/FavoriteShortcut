//! アプリ全体の流れ（C# 版 App.xaml.cs に相当）。
//!
//! winit のイベントループで、管理画面・ランチャー・ダイアログの各ウィンドウを動かす。
//! 画面の部品は「やりたいこと」を Command として積み、ここでウィンドウの開閉や
//! 確認のメッセージ・ファイル選択などをまとめて行う（描画の途中で別の画面を出さないため）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::icons::IconService;
use crate::launch::{self, LaunchResult};
use crate::loc;
use crate::model::{AppLanguage, AppSettings, AppTheme, Shortcut, TargetType};
use crate::platform::{self, HotKeyWindow, MsgIcon, PlatformEvent};
use crate::search::SearchCache;
use crate::store::Store;
use crate::target;
use crate::transfer::{self, AutoBackup};
use crate::tray::{Tray, TrayAction};
use crate::ui::dialogs::{self, Dialog, TextPurpose};
use crate::ui::host::{Gpu, WindowHost};
use crate::ui::launcher::{self, LauncherState};
use crate::ui::main_window::{self, MainState};
use crate::ui::style::{self, Palette};

/// 裏の処理やトレイ・ホットキーから、画面のスレッドへ送る知らせ。
pub enum UserEvent {
    Platform(PlatformEvent),
    Tray(TrayAction),
    /// 裏の処理（アイコンの取得など）が終わった
    Wake,
    /// 管理画面への外からのドラッグ＆ドロップ（エクスプローラーのファイル、ブラウザの URL など）
    Drop(crate::dragdrop::DropEvent),
    /// from_main: 管理画面から開いた（失敗したときの案内を変える）
    LaunchFinished {
        id: Option<String>,
        result: LaunchResult,
        reveal: bool,
        from_main: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Main,
    /// 持ち主なし（他のアプリを使っている途中に、登録画面だけを前に出すとき）
    None,
}

/// 画面の部品から、アプリ本体に頼むこと。
pub enum Command {
    OpenEdit { existing: Option<String>, folder: Option<String>, initial: Option<String>, owner: Owner },
    OpenSettings,
    OpenBookmarkImport { folder: Option<String> },
    OpenTagManager,
    OpenMoveToFolder { ids: Vec<String> },
    OpenFolderDelete { folder: String },
    OpenTextInput { purpose: TextPurpose, title: String, label: String, initial: String },
    Export,
    Import,
    BackupNow,
    OpenDataFolder,
    ShowHelp,
    ShowLauncher,
    HideLauncher,
    Launch { ids: Vec<String>, from_main: bool },
    Reveal { id: String },
    DeleteShortcuts { ids: Vec<String> },
    DeleteFolderConfirmed { folder: String, delete_shortcuts: bool },
    RegisterFromClipboard,
    AddPaths { paths: Vec<String>, folder: Option<String> },
    RefreshIcons { ids: Vec<String> },
    CloseDialog { window: WindowId },
    Message { text: String, title: String, icon: MsgIcon },
    PreviewTheme(AppTheme),
    PreviewLanguage(AppLanguage),
    SettingsSaved { previous: AppSettings },
    SettingsCancelled,
    RefreshTray,
}

/// 画面の部品が共有するもの。
pub struct Core {
    pub store: Store,
    pub settings: AppSettings,
    pub icons: IconService,
    pub search: SearchCache,
    pub palette: Palette,
    pub commands: Vec<Command>,
    /// ステータス欄に一時的に出す文字（「コピーしました」など）
    pub status_override: Option<String>,
    pub hotkeys: Option<HotKeyWindow>,
    pub hotkey_failed: bool,
    pub clipboard_hotkey_failed: bool,
    pub proxy: EventLoopProxy<UserEvent>,
    /// 起動したショートカットのアイコンを取り直す予定（時刻, ID）
    rechecks: Vec<(Instant, String)>,
    /// ランチャーを最後に隠した時刻（隠した直後のホットキーで再表示しないため）
    pub launcher_hidden_at: Option<Instant>,
}

impl Core {
    pub fn push(&mut self, command: Command) {
        self.commands.push(command);
    }

    pub fn is_dark(&self) -> bool {
        self.palette.dark
    }

    pub fn status(&mut self, text: String) {
        self.status_override = Some(text);
    }

    pub fn save_settings(&mut self, settings: AppSettings) {
        if let Err(e) = crate::settings::save(&self.store, &settings) {
            crate::log::warn_err("設定の保存に失敗しました。", &e);
        }
        self.settings = settings;
    }

    /// 最近使った項目（トレイのメニュー用）: (ID, タイトル, 対象)
    pub fn recent_items(&self, count: usize) -> Vec<(String, String, String)> {
        crate::search::recent(self.store.shortcuts(), count)
            .into_iter()
            .filter_map(|id| self.store.find_shortcut(&id).map(|s| (s.id.clone(), s.display_title().to_string(), s.target.clone())))
            .collect()
    }
}

struct DialogWin {
    host: WindowHost,
    owner: Owner,
    dialog: Dialog,
}

pub struct App {
    core: Core,
    gpu: Option<Gpu>,
    main: Option<(WindowHost, MainState)>,
    launcher: Option<(WindowHost, LauncherState)>,
    dialogs: Vec<DialogWin>,
    tray: Option<Tray>,
    auto_backup: AutoBackup,
    start_hidden: bool,
    started: bool,
    exiting: bool,
}

pub fn system_dark(theme: AppTheme) -> bool {
    match theme {
        AppTheme::Dark => true,
        AppTheme::Light => false,
        AppTheme::Auto => windows_uses_dark_mode(),
    }
}

/// Windows のアプリの配色がダークか（読めなければライト）。
pub fn windows_uses_dark_mode() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    }
    .is_ok();
    ok && value == 0
}

impl App {
    pub fn new(store: Store, settings: AppSettings, proxy: EventLoopProxy<UserEvent>, start_hidden: bool) -> Self {
        let wake_proxy = proxy.clone();
        let icons = IconService::new(Arc::new(move || {
            let _ = wake_proxy.send_event(UserEvent::Wake);
        }));
        let palette = if system_dark(settings.theme) { style::DARK } else { style::LIGHT };
        Self {
            core: Core {
                store,
                settings,
                icons,
                search: SearchCache::default(),
                palette,
                commands: Vec::new(),
                status_override: None,
                hotkeys: None,
                hotkey_failed: false,
                clipboard_hotkey_failed: false,
                proxy,
                rechecks: Vec::new(),
                launcher_hidden_at: None,
            },
            gpu: None,
            main: None,
            launcher: None,
            dialogs: Vec::new(),
            tray: None,
            auto_backup: AutoBackup::default(),
            start_hidden,
            started: false,
            exiting: false,
        }
    }

    // ---------------------------------------------------------- 起動処理

    fn start(&mut self, el: &ActiveEventLoop) {
        // ランチャーは最初に作って隠しておく（ホットキーで一瞬で出せるように）
        let launcher_window = match create_launcher_window(el) {
            Ok(w) => w,
            Err(e) => {
                fatal(&format!("ウィンドウを作成できませんでした: {e}"));
                el.exit();
                return;
            }
        };
        let (gpu, surface) = match Gpu::create(launcher_window.clone()) {
            Ok(v) => v,
            Err(e) => {
                fatal(&format!("画面を描画できませんでした（GPU を初期化できません）: {e}"));
                el.exit();
                return;
            }
        };
        let palette = self.core.palette;
        match WindowHost::new(&gpu, el, launcher_window, Some(surface), |ctx| style::apply(ctx, &palette)) {
            Ok(mut host) => {
                host.auto_height = true;
                self.launcher = Some((host, LauncherState::default()));
            }
            Err(e) => crate::log::warn(&format!("ランチャーを作成できませんでした: {e}")),
        }
        self.gpu = Some(gpu);

        // ホットキーと二重起動の通知の窓口
        let proxy = self.core.proxy.clone();
        self.core.hotkeys = HotKeyWindow::create(move |e| {
            let _ = proxy.send_event(UserEvent::Platform(e));
        });
        self.register_hotkeys();

        // トレイ
        let proxy = self.core.proxy.clone();
        self.tray = Tray::create(&self.core.recent_items(10), move |action| {
            let _ = proxy.send_event(UserEvent::Tray(action));
        });

        if !self.start_hidden {
            self.show_main(el);
        }
        crate::log::info(&format!("起動しました（Rust 版、データ保存場所: {}）", crate::paths::data_directory().display()));
        self.open_dev_screens();
    }

    /// 開発・確認用: 保存場所を環境変数で指定して動かしているときだけ、
    /// FAVORITESHORTCUT_DEV_SHOW に書いた画面を起動直後に開く（画面の見た目の確認に使う）。
    fn open_dev_screens(&mut self) {
        if !crate::paths::is_overridden_by_environment() {
            return;
        }
        let Ok(list) = std::env::var("FAVORITESHORTCUT_DEV_SHOW") else { return };
        let first_item = self.core.store.shortcuts().first().map(|s| s.id.clone());
        let first_folder = self.core.store.root_folders().first().cloned();
        for name in list.split(',').map(str::trim) {
            let command = match name {
                "launcher" => Command::ShowLauncher,
                "settings" => Command::OpenSettings,
                "bookmarks" => Command::OpenBookmarkImport { folder: None },
                "tags" => Command::OpenTagManager,
                "new" => Command::OpenEdit {
                    existing: None,
                    folder: first_folder.clone(),
                    initial: Some("https://www.example.com/".into()),
                    owner: Owner::Main,
                },
                "edit" => Command::OpenEdit { existing: first_item.clone(), folder: None, initial: None, owner: Owner::Main },
                "move" => Command::OpenMoveToFolder { ids: first_item.iter().cloned().collect() },
                "folderdelete" => match &first_folder {
                    Some(f) => Command::OpenFolderDelete { folder: f.clone() },
                    None => continue,
                },
                "text" => Command::OpenTextInput {
                    purpose: TextPurpose::NewFolder { parent: None },
                    title: loc::t("Str.Folder.NewTitle"),
                    label: loc::t("Str.Folder.NameLabel"),
                    initial: String::new(),
                },
                _ => continue,
            };
            self.core.push(command);
        }
    }

    fn register_hotkeys(&mut self) {
        // 開発・確認用に保存場所を環境変数で指定しているときは、普段使いのアプリの邪魔をしないよう
        // グローバルホットキーを登録しない（FAVORITESHORTCUT_DEV_HOTKEYS=1 のときだけ登録する）
        if !hotkeys_allowed() {
            crate::log::info("開発用の保存場所で起動しているため、ホットキーは登録しません。");
            return;
        }
        let s = self.core.settings.clone();
        if let Some(h) = self.core.hotkeys.as_mut() {
            self.core.hotkey_failed = !h.register_launcher(s.hotkey_modifiers, s.hotkey_key);
            self.core.clipboard_hotkey_failed = !h.register_clipboard(s.clipboard_hotkey_modifiers, s.clipboard_hotkey_key);
        }
    }

    // ---------------------------------------------------------- ウィンドウ

    fn show_main(&mut self, el: &ActiveEventLoop) {
        let mut created = false;
        if self.main.is_none() {
            let Some(gpu) = self.gpu.as_ref() else { return };
            let s = &self.core.settings;
            let window = match create_main_window(el, s, self.core.is_dark()) {
                Ok(w) => w,
                Err(e) => {
                    crate::log::error(&format!("管理画面を作成できませんでした: {e}"));
                    return;
                }
            };
            let palette = self.core.palette;
            match WindowHost::new(gpu, el, window, None, |ctx| style::apply(ctx, &palette)) {
                Ok(host) => {
                    // ドラッグ＆ドロップを受け付ける（ドロップした位置と、URL などの文字列も扱えるよう自前で）
                    if let Some(hwnd) = host.hwnd() {
                        let proxy = self.core.proxy.clone();
                        crate::dragdrop::register(hwnd, move |e| {
                            let _ = proxy.send_event(UserEvent::Drop(e));
                        });
                    }
                    let state = MainState::new(&self.core);
                    self.main = Some((host, state));
                    created = true;
                }
                Err(e) => {
                    crate::log::error(&format!("管理画面を作成できませんでした: {e}"));
                    return;
                }
            }
        }
        if let Some((_, state)) = self.main.as_mut() {
            state.on_shown();
        }
        if created && let Some(id) = self.main.as_ref().map(|(h, _)| h.id()) {
            // 描いてから出す（真っ白な画面が一瞬見えないように）
            self.redraw(id);
        }
        if let Some((host, _)) = self.main.as_mut() {
            if host.window.is_minimized() == Some(true) {
                host.window.set_minimized(false);
            }
            show_and_activate(host);
        }
    }

    fn hide_main(&mut self) {
        if let Some((host, state)) = self.main.as_mut() {
            state.persist(&mut self.core, &host.window);
            host.window.set_visible(false);
        }
    }

    fn main_hwnd(&self) -> Option<windows::Win32::Foundation::HWND> {
        self.main.as_ref().filter(|(h, _)| h.window.is_visible() == Some(true)).and_then(|(h, _)| h.hwnd())
    }

    fn toggle_launcher(&mut self) {
        let visible = self.launcher.as_ref().is_some_and(|(h, _)| h.window.is_visible() == Some(true));
        if visible {
            self.hide_launcher();
        } else {
            self.show_launcher();
        }
    }

    fn show_launcher(&mut self) {
        let Some((host, state)) = self.launcher.as_mut() else { return };
        state.reset(&mut self.core);
        let id = host.id();
        // 描いて高さを決めてから出す（出たあとに大きさが変わる様子が見えないように）
        for _ in 0..3 {
            if !self.redraw(id) {
                break;
            }
        }
        let Some((host, _)) = self.launcher.as_mut() else { return };
        // マウスカーソルのあるモニターの中央やや上に出す（§65: マルチモニター対応）
        if let Some((left, top, width, height)) = platform::cursor_monitor_work_area() {
            let scale = host.window.scale_factor();
            let w = (launcher::WIDTH as f64 * scale) as i32;
            let current_h = host.window.outer_size().height as i32;
            let x = left + (width - w) / 2;
            let y = top + ((height as f64 * 0.22) - current_h as f64 * 0.15).max(0.0) as i32;
            host.window.set_outer_position(PhysicalPosition::new(x, y));
        }
        show_and_activate(host);
    }

    fn hide_launcher(&mut self) {
        if let Some((host, state)) = self.launcher.as_mut()
            && host.window.is_visible() == Some(true)
        {
            host.window.set_visible(false);
            state.clear();
            self.core.launcher_hidden_at = Some(Instant::now());
        }
    }

    fn open_dialog(&mut self, el: &ActiveEventLoop, dialog: Dialog, owner: Owner) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        // 持ち主: 開いているダイアログがあればその上、なければ管理画面
        let owner_hwnd = match owner {
            Owner::Main => self.dialogs.last().and_then(|d| d.host.hwnd()).or_else(|| self.main.as_ref().and_then(|(h, _)| h.hwnd())),
            Owner::None => None,
        };
        let (title, width, height) = dialog.window_spec();
        let window = match create_dialog_window(el, &title, width, height, owner_hwnd, self.core.is_dark()) {
            Ok(w) => w,
            Err(e) => {
                crate::log::error(&format!("画面を作成できませんでした: {e}"));
                return;
            }
        };
        let palette = self.core.palette;
        let Ok(mut host) = WindowHost::new(gpu, el, window, None, |ctx| style::apply(ctx, &palette)) else { return };
        host.auto_height = height.is_none();
        if let Some(owner) = owner_hwnd {
            platform::enable_window(owner, false);
        }
        let id = host.id();
        self.dialogs.push(DialogWin { host, owner, dialog });
        self.present_dialog(id, owner_hwnd);
    }

    /// 描いて大きさを決めてから、持ち主の中央に出す（真っ白な画面や大きさが変わる様子が見えないように）。
    fn present_dialog(&mut self, window_id: WindowId, owner: Option<windows::Win32::Foundation::HWND>) {
        for _ in 0..3 {
            if !self.redraw(window_id) {
                break;
            }
        }
        let Some(d) = self.dialogs.iter_mut().find(|d| d.host.id() == window_id) else { return };
        let size = d.host.window.outer_size();
        let (w, h) = (size.width as i32, size.height as i32);
        let area = owner.and_then(platform::monitor_work_area_of).or_else(platform::cursor_monitor_work_area);
        let mut pos = match (owner.and_then(platform::window_rect), area) {
            (Some((l, t, r, b)), _) => (l + (r - l - w) / 2, t + (b - t - h) / 2),
            (None, Some((l, t, aw, ah))) => (l + (aw - w) / 2, t + (ah - h) / 2),
            (None, None) => (100, 100),
        };
        // 画面の外にはみ出さないように
        if let Some((l, t, aw, ah)) = area {
            pos.0 = pos.0.clamp(l, (l + aw - w).max(l));
            pos.1 = pos.1.clamp(t, (t + ah - h).max(t));
        }
        d.host.window.set_outer_position(PhysicalPosition::new(pos.0, pos.1));
        show_and_activate(&mut d.host);
    }

    fn close_dialog(&mut self, window: WindowId) {
        let Some(index) = self.dialogs.iter().position(|d| d.host.id() == window) else { return };
        let closing = self.dialogs.remove(index);

        // 持ち主を先に操作できるように戻してから閉じる（別のアプリへフォーカスが飛ばないように）
        let owner_hwnd = match closing.owner {
            Owner::Main => self.dialogs.last().and_then(|d| d.host.hwnd()).or_else(|| self.main.as_ref().and_then(|(h, _)| h.hwnd())),
            Owner::None => None,
        };
        if let Some(owner) = owner_hwnd {
            platform::enable_window(owner, true);
            if !dev_quiet() {
                platform::force_foreground(owner);
            }
        }
        let outcome = closing.dialog.into_outcome();
        drop(closing.host);
        self.handle_dialog_outcome(outcome);
    }

    fn handle_dialog_outcome(&mut self, outcome: dialogs::Outcome) {
        match outcome {
            dialogs::Outcome::None => {}
            dialogs::Outcome::TextEntered { purpose, value } => self.handle_text_input(purpose, value),
            dialogs::Outcome::MoveTo { ids, folder } => {
                self.core.store.begin_batch();
                for id in &ids {
                    if let Err(e) = self.core.store.move_shortcut(id, folder.as_deref()) {
                        crate::log::warn_err("移動に失敗しました。", &e);
                    }
                }
                self.core.store.end_batch();
            }
            dialogs::Outcome::DeleteFolder { folder, delete_shortcuts } => {
                self.core.push(Command::DeleteFolderConfirmed { folder, delete_shortcuts });
            }
            dialogs::Outcome::Import { path, mode } => self.run_import(&path, mode),
            dialogs::Outcome::BookmarksImported { imported, skipped } => {
                let skipped_text = if skipped > 0 { loc::tf("Str.Common.SkippedDuplicates", &[&skipped]) } else { String::new() };
                self.message(&loc::tf("Str.Bookmark.Done", &[&imported, &skipped_text]), &loc::t("Str.Bookmark.DoneTitle"), MsgIcon::Info);
            }
        }
        self.request_repaint_all();
    }

    fn handle_text_input(&mut self, purpose: TextPurpose, value: String) {
        match purpose {
            TextPurpose::NewFolder { parent } => match self.core.store.create_folder(&value, parent.as_deref()) {
                Ok(id) => {
                    if let Some((_, state)) = self.main.as_mut() {
                        state.select_folder(Some(id));
                    }
                }
                Err(e) => crate::log::warn_err("フォルダを作成できませんでした。", &e),
            },
            TextPurpose::RenameFolder { id } => {
                if let Err(e) = self.core.store.rename_folder(&id, &value) {
                    crate::log::warn_err("フォルダ名を変更できませんでした。", &e);
                }
            }
            TextPurpose::RenameTag { old } => {
                if let Err(e) = self.core.store.rename_tag(&old, &value) {
                    crate::log::warn_err("タグ名を変更できませんでした。", &e);
                }
                for d in &mut self.dialogs {
                    d.dialog.on_tags_changed(&self.core);
                }
            }
            TextPurpose::NewFolderForDialog { parent, dialog } => match self.core.store.create_folder(&value, parent.as_deref()) {
                Ok(id) => {
                    if let Some(d) = self.dialogs.iter_mut().find(|d| d.host.id() == dialog) {
                        d.dialog.on_folder_created(&self.core, id);
                    }
                }
                Err(e) => crate::log::warn_err("フォルダを作成できませんでした。", &e),
            },
        }
    }

    fn message(&self, text: &str, title: &str, icon: MsgIcon) {
        let owner = self.dialogs.last().and_then(|d| d.host.hwnd()).or_else(|| self.main_hwnd());
        platform::message(owner, text, title, icon);
    }

    fn confirm(&self, text: &str, title: &str, icon: MsgIcon) -> bool {
        let owner = self.dialogs.last().and_then(|d| d.host.hwnd()).or_else(|| self.main_hwnd());
        platform::confirm(owner, text, title, icon)
    }

    fn request_repaint_all(&mut self) {
        if let Some((h, _)) = self.main.as_mut() {
            h.request_repaint();
        }
        if let Some((h, _)) = self.launcher.as_mut() {
            h.request_repaint();
        }
        for d in &mut self.dialogs {
            d.host.request_repaint();
        }
    }

    fn apply_palette(&mut self) {
        let dark = system_dark(self.core.settings.theme);
        self.apply_palette_for(dark);
    }

    fn apply_palette_for(&mut self, dark: bool) {
        self.core.palette = if dark { style::DARK } else { style::LIGHT };
        let palette = self.core.palette;
        let theme = Some(if dark { winit::window::Theme::Dark } else { winit::window::Theme::Light });
        let mut hosts: Vec<&mut WindowHost> = Vec::new();
        if let Some((h, _)) = self.main.as_mut() {
            hosts.push(h);
        }
        if let Some((h, _)) = self.launcher.as_mut() {
            hosts.push(h);
        }
        for d in &mut self.dialogs {
            hosts.push(&mut d.host);
        }
        for h in hosts {
            style::apply(&h.ctx, &palette);
            h.window.set_theme(theme);
            h.request_repaint();
        }
    }

    fn apply_language(&mut self, language: AppLanguage) {
        if loc::apply(language) {
            self.core.store.refresh_localized_texts();
            self.rebuild_tray();
            if let Some((h, _)) = self.main.as_ref() {
                h.window.set_title(&loc::t("Str.App.Name"));
            }
            if let Some((h, _)) = self.launcher.as_ref() {
                h.window.set_title(&loc::t("Str.Launcher.Title"));
            }
            for d in &self.dialogs {
                d.host.window.set_title(&d.dialog.window_spec().0);
            }
            self.request_repaint_all();
        }
    }

    fn rebuild_tray(&self) {
        if let Some(t) = self.tray.as_ref() {
            t.rebuild(&self.core.recent_items(10));
        }
    }

    // ---------------------------------------------------------------- 操作

    fn process_commands(&mut self, el: &ActiveEventLoop) {
        let mut guard = 0;
        while !self.core.commands.is_empty() && guard < 100 {
            guard += 1;
            let commands = std::mem::take(&mut self.core.commands);
            for command in commands {
                self.run_command(el, command);
            }
        }
    }

    fn run_command(&mut self, el: &ActiveEventLoop, command: Command) {
        match command {
            Command::OpenEdit { existing, folder, initial, owner } => {
                if owner == Owner::Main {
                    self.show_main(el);
                }
                let dialog = Dialog::edit(&mut self.core, existing, folder, initial);
                self.open_dialog(el, dialog, owner);
            }
            Command::OpenSettings => {
                self.show_main(el);
                let dialog = Dialog::settings(&self.core);
                self.open_dialog(el, dialog, Owner::Main);
            }
            Command::OpenBookmarkImport { folder } => {
                let dialog = Dialog::bookmarks(&self.core, folder);
                self.open_dialog(el, dialog, Owner::Main);
            }
            Command::OpenTagManager => {
                let dialog = Dialog::tag_manager(&self.core);
                self.open_dialog(el, dialog, Owner::Main);
            }
            Command::OpenMoveToFolder { ids } => {
                let current = ids.first().and_then(|id| self.core.store.find_shortcut(id)).and_then(|s| s.folder_id.clone());
                let dialog = Dialog::folder_picker(ids, current);
                self.open_dialog(el, dialog, Owner::Main);
            }
            Command::OpenFolderDelete { folder } => {
                let dialog = Dialog::folder_delete(&self.core, folder);
                self.open_dialog(el, dialog, Owner::Main);
            }
            Command::OpenTextInput { purpose, title, label, initial } => {
                let dialog = Dialog::text_input(purpose, title, label, initial);
                self.open_dialog(el, dialog, Owner::Main);
            }
            Command::CloseDialog { window } => self.close_dialog(window),
            Command::Export => self.run_export(),
            Command::Import => self.choose_import(el),
            Command::BackupNow => match transfer::create_backup(&self.core.store, "manual") {
                Ok(path) => self.message(&loc::tf("Str.Backup.Done", &[&path.display()]), &loc::t("Str.Backup.Title"), MsgIcon::Info),
                Err(e) => {
                    crate::log::error_err("バックアップに失敗しました。", &e);
                    self.message(&loc::tf("Str.Backup.Failed", &[&e]), &loc::t("Str.Common.Error"), MsgIcon::Error);
                }
            },
            Command::OpenDataFolder => launch::open_path(crate::paths::data_directory().to_string_lossy()),
            Command::ShowHelp => {
                // README は日本語のみなので、英語表示のときはアプリ内の説明を出す
                let readme = crate::paths::exe_directory().join("README.md");
                if loc::current() == AppLanguage::Japanese && readme.is_file() {
                    launch::open_path(readme.to_string_lossy());
                } else {
                    self.message(&loc::t("Str.Help.Body"), &loc::t("Str.Help.Title"), MsgIcon::Info);
                }
            }
            Command::ShowLauncher => self.show_launcher(),
            Command::HideLauncher => self.hide_launcher(),
            Command::Launch { ids, from_main } => {
                for id in ids {
                    let Some(item) = self.core.store.find_shortcut(&id) else { continue };
                    let (target, kind) = (item.target.clone(), item.target_type);
                    let proxy = self.core.proxy.clone();
                    let id2 = id.clone();
                    // 開く処理は裏で行う（ネットワークの状態が悪くても、画面やホットキーが止まらないように）
                    launch::launch_async(target, kind, move |result| {
                        let _ = proxy.send_event(UserEvent::LaunchFinished { id: Some(id2), result, reveal: false, from_main });
                    });
                }
            }
            Command::Reveal { id } => {
                let Some(item) = self.core.store.find_shortcut(&id) else { return };
                let (target, kind) = (item.target.clone(), item.target_type);
                let proxy = self.core.proxy.clone();
                launch::reveal_async(target, kind, move |result| {
                    let _ = proxy.send_event(UserEvent::LaunchFinished { id: None, result, reveal: true, from_main: false });
                });
            }
            Command::DeleteShortcuts { ids } => self.delete_shortcuts(&ids),
            Command::DeleteFolderConfirmed { folder, delete_shortcuts } => {
                if let Err(e) = self.core.store.delete_folder(&folder, delete_shortcuts) {
                    crate::log::warn_err("フォルダを削除できませんでした。", &e);
                }
                if let Some((_, state)) = self.main.as_mut() {
                    state.select_folder(None);
                }
            }
            Command::RegisterFromClipboard => self.register_from_clipboard(el),
            Command::AddPaths { paths, folder } => self.add_paths(el, paths, folder),
            Command::RefreshIcons { ids } => self.refresh_icons(&ids),
            Command::Message { text, title, icon } => self.message(&text, &title, icon),
            Command::PreviewTheme(theme) => self.apply_palette_for(system_dark(theme)),
            Command::PreviewLanguage(language) => self.apply_language(language),
            Command::SettingsSaved { previous } => self.after_settings_saved(previous),
            Command::SettingsCancelled => {
                // プレビュー用に変えていたテーマと言語を元に戻す
                self.apply_palette();
                let language = self.core.settings.language;
                self.apply_language(language);
            }
            Command::RefreshTray => self.rebuild_tray(),
        }
    }

    fn after_settings_saved(&mut self, previous: AppSettings) {
        let s = self.core.settings.clone();
        self.apply_palette();
        self.apply_language(s.language);

        // 2 つのキーを入れ替えた場合に備え、先に「クリップボードから登録」を解除してから登録し直す
        if hotkeys_allowed()
            && let Some(h) = self.core.hotkeys.as_mut()
        {
            h.register_clipboard(0, 0);
            if !h.register_launcher(s.hotkey_modifiers, s.hotkey_key) {
                self.core.hotkey_failed = true;
                let text = loc::tf("Str.Settings.HotKeyFailed", &[&crate::keys::describe(s.hotkey_modifiers, s.hotkey_key)]);
                self.message(&text, &loc::t("Str.Settings.HotKeyTitle"), MsgIcon::Warning);
            } else {
                self.core.hotkey_failed = false;
            }
            if let Some(h) = self.core.hotkeys.as_mut()
                && !h.register_clipboard(s.clipboard_hotkey_modifiers, s.clipboard_hotkey_key)
            {
                self.core.clipboard_hotkey_failed = true;
                let text = loc::tf(
                    "Str.Settings.ClipboardHotKeyFailed",
                    &[&crate::keys::describe(s.clipboard_hotkey_modifiers, s.clipboard_hotkey_key)],
                );
                self.message(&text, &loc::t("Str.Settings.HotKeyTitle"), MsgIcon::Warning);
            } else {
                self.core.clipboard_hotkey_failed = false;
            }
        }

        if platform::startup_enabled() != s.run_at_startup && !platform::set_startup_enabled(s.run_at_startup) {
            self.message(&loc::t("Str.Settings.StartupFailed"), &loc::t("Str.Settings.StartupTitle"), MsgIcon::Warning);
        }
        if previous.view_mode != s.view_mode || previous.icon_size != s.icon_size {
            self.request_repaint_all();
        }
    }

    fn delete_shortcuts(&mut self, ids: &[String]) {
        let items: Vec<Shortcut> = ids.iter().filter_map(|id| self.core.store.find_shortcut(id).cloned()).collect();
        if items.is_empty() {
            return;
        }
        let text = if items.len() == 1 {
            loc::tf("Str.Common.ConfirmDeleteOne", &[&items[0].display_title()])
        } else {
            loc::tf("Str.Main.DeleteMany", &[&items.len()])
        };
        let text = format!("{text}\n\n{}", loc::t("Str.Common.CannotUndo"));
        if !self.confirm(&text, &loc::t("Str.Common.ConfirmDeleteTitle"), MsgIcon::Warning) {
            return;
        }
        // 画面の更新は最後の 1 回にまとめる
        self.core.store.begin_batch();
        for item in &items {
            self.core.icons.delete_custom_icon(item.icon_path.as_deref());
            if let Err(e) = self.core.store.delete_shortcut(&item.id) {
                crate::log::warn_err("削除に失敗しました。", &e);
            }
        }
        self.core.store.end_batch();
        self.rebuild_tray();
    }

    /// コピーした URL / パスを入れた状態で登録画面を開く（トレイ・ランチャー・ホットキーから）。
    fn register_from_clipboard(&mut self, el: &ActiveEventLoop) {
        // ホットキーを続けて押したときは、開いている登録画面を前に出すだけにする
        if let Some(d) = self.dialogs.iter_mut().find(|d| d.dialog.is_edit()) {
            show_and_activate(&mut d.host);
            return;
        }
        let main_visible =
            self.main.as_ref().is_some_and(|(h, _)| h.window.is_visible() == Some(true) && h.window.is_minimized() != Some(true));
        let folder = if main_visible { self.main.as_ref().and_then(|(_, s)| s.current_folder_id()) } else { None };
        let targets = platform::clipboard_targets();
        if targets.len() > 1 {
            // 複数のファイルは、管理画面へドロップしたときと同じ流れで登録する
            self.show_main(el);
            self.add_paths(el, targets, folder);
            return;
        }
        let owner = if main_visible { Owner::Main } else { Owner::None };
        let dialog = Dialog::edit(&mut self.core, None, folder, targets.into_iter().next());
        self.open_dialog(el, dialog, owner);
    }

    /// エクスプローラーからドロップされたファイル / フォルダをショートカットとして登録する（§18）。
    fn add_paths(&mut self, el: &ActiveEventLoop, paths: Vec<String>, folder: Option<String>) {
        if paths.is_empty() {
            return;
        }
        // 1 件だけなら内容を確認・編集できるようダイアログを出す
        if paths.len() == 1 {
            let dialog = Dialog::edit(&mut self.core, None, folder, paths.into_iter().next());
            self.open_dialog(el, dialog, Owner::Main);
            return;
        }
        if !self.confirm(&loc::tf("Str.Main.AddMany", &[&paths.len()]), &loc::t("Str.Main.AddManyTitle"), MsgIcon::Question) {
            return;
        }
        self.core.store.begin_batch();
        for path in paths {
            let kind = target::detect(&path);
            let item = Shortcut {
                title: target::suggest_title(&path, kind),
                target: path,
                target_type: kind,
                folder_id: folder.clone(),
                ..Shortcut::new()
            };
            if let Err(e) = self.core.store.add_shortcut(item) {
                crate::log::warn_err("登録に失敗しました。", &e);
            }
        }
        self.core.store.end_batch();
    }

    fn refresh_icons(&mut self, ids: &[String]) {
        let dark = self.core.is_dark();
        let use_cache = self.core.settings.use_browser_icon_cache;
        let mut requested = 0;
        for id in ids {
            let Some(item) = self.core.store.find_shortcut(id).cloned() else { continue };
            if item.target_type == TargetType::Web {
                self.core.icons.forget_failure(&item);
                if self.core.icons.request_favicon(&item, true, use_cache, dark) {
                    requested += 1;
                } else if !use_cache {
                    self.core.status(loc::t("Str.Main.BrowserIconCacheOff"));
                }
            } else {
                self.core.icons.clear_memory_cache();
            }
        }
        if requested == 0 {
            self.request_repaint_all();
        }
    }

    fn run_export(&mut self) {
        let owner = self.main.as_ref().map(|(h, _)| h.window.clone());
        let mut dialog = rfd::FileDialog::new()
            .set_title(loc::t("Str.Export.DialogTitle"))
            .set_file_name(transfer::suggested_file_name())
            .add_filter(zip_filter_name(), &["zip"]);
        if let Some(w) = owner.as_ref() {
            dialog = dialog.set_parent(w.as_ref());
        }
        let Some(path) = dialog.save_file() else { return };
        match transfer::export(&self.core.store, &self.core.settings, &path) {
            Ok(()) => self.message(&loc::tf("Str.Export.Done", &[&path.display()]), &loc::t("Str.Export.DoneTitle"), MsgIcon::Info),
            Err(e) => {
                crate::log::error_err("エクスポートに失敗しました。", &e);
                self.message(&loc::tf("Str.Export.Failed", &[&e]), &loc::t("Str.Common.Error"), MsgIcon::Error);
            }
        }
    }

    fn choose_import(&mut self, el: &ActiveEventLoop) {
        let owner = self.main.as_ref().map(|(h, _)| h.window.clone());
        let mut dialog = rfd::FileDialog::new().set_title(loc::t("Str.Import.DialogTitle")).add_filter(zip_filter_name(), &["zip"]);
        if let Some(w) = owner.as_ref() {
            dialog = dialog.set_parent(w.as_ref());
        }
        let Some(path) = dialog.pick_file() else { return };
        let Some(manifest) = transfer::peek_manifest(&path) else {
            self.message(&loc::t("Str.Import.NotOurFile"), &loc::t("Str.Import.CannotImportTitle"), MsgIcon::Warning);
            return;
        };
        let dialog = Dialog::import_mode(path, manifest);
        self.open_dialog(el, dialog, Owner::Main);
    }

    fn run_import(&mut self, path: &std::path::Path, mode: transfer::ImportMode) {
        match transfer::import(&mut self.core.store, path, mode) {
            Ok(summary) => {
                self.core.settings = crate::settings::load(&self.core.store);
                self.core.icons.clear_memory_cache();
                if let Some((_, state)) = self.main.as_mut() {
                    state.select_folder(None);
                }
                self.apply_palette();
                let language = self.core.settings.language;
                self.apply_language(language);
                self.rebuild_tray();
                let skipped =
                    if summary.skipped > 0 { loc::tf("Str.Common.SkippedDuplicates", &[&summary.skipped]) } else { String::new() };
                let text = loc::tf("Str.Import.Done", &[&summary.folders, &summary.shortcuts, &skipped, &summary.backup_path.display()]);
                self.message(&text, &loc::t("Str.Import.DoneTitle"), MsgIcon::Info);
            }
            Err(e) => {
                crate::log::error_err("インポートに失敗しました。", &e);
                self.message(&loc::tf("Str.Import.Failed", &[&e]), &loc::t("Str.Common.Error"), MsgIcon::Error);
            }
        }
    }

    fn on_launch_finished(&mut self, id: Option<String>, result: LaunchResult, reveal: bool, from_main: bool) {
        if self.exiting {
            return;
        }
        if !result.success {
            // 管理画面から開いた場合は編集画面で直せることを、ランチャーやトレイからなら残っていることを伝える
            let kept = if reveal {
                String::new()
            } else if from_main {
                format!("\n\n{}", loc::t("Str.Launch.KeptEditHint"))
            } else {
                format!("\n\n{}", loc::t("Str.Launch.Kept"))
            };
            let text = format!("{}\n\n{}{kept}", result.error_title, result.error_detail);
            self.message(&text, &loc::t("Str.App.Name"), MsgIcon::Warning);
            return;
        }
        let Some(id) = id else { return };
        let Some(item) = self.core.store.find_shortcut(&id).cloned() else { return }; // 開いている間に削除された
        if let Err(e) = self.core.store.record_usage(&id) {
            crate::log::warn_err("使用履歴を記録できませんでした。", &e);
        }
        // ブラウザで開くと favicon がブラウザ側にキャッシュされるので、少し待ってから取り直す
        if item.target_type == TargetType::Web && item.icon_path.is_none() && self.core.settings.use_browser_icon_cache {
            self.core.icons.forget_failure(&item);
            let now = Instant::now();
            self.core.rechecks.push((now + Duration::from_secs(5), id.clone()));
            self.core.rechecks.push((now + Duration::from_secs(20), id));
        }
        self.rebuild_tray();
    }

    fn process_rechecks(&mut self) {
        let now = Instant::now();
        let due: Vec<String> = self.core.rechecks.iter().filter(|(t, _)| *t <= now).map(|(_, id)| id.clone()).collect();
        self.core.rechecks.retain(|(t, _)| *t > now);
        let dark = self.core.is_dark();
        let use_cache = self.core.settings.use_browser_icon_cache;
        for id in due {
            let Some(item) = self.core.store.find_shortcut(&id).cloned() else { continue };
            if item.icon_path.is_some() {
                continue;
            }
            self.core.icons.forget_failure(&item);
            self.core.icons.request_favicon(&item, false, use_cache, dark);
        }
    }

    fn exit(&mut self, el: &ActiveEventLoop) {
        if self.exiting {
            return;
        }
        self.exiting = true;
        if let Some((host, state)) = self.main.as_mut() {
            state.persist(&mut self.core, &host.window);
            if let Some(hwnd) = host.hwnd() {
                crate::dragdrop::revoke(hwnd);
            }
        }
        self.tray = None;
        self.core.hotkeys = None;
        self.dialogs.clear();
        self.launcher = None;
        self.main = None;
        crate::log::info("終了しました。");
        el.exit();
    }

    // ---------------------------------------------------------- 描画

    /// 1 フレーム描く。中身に合わせてウィンドウの高さを変えたら true。
    fn redraw(&mut self, window_id: WindowId) -> bool {
        let Some(gpu) = self.gpu.as_ref() else { return false };
        let core = &mut self.core;

        if let Some((host, state)) = self.main.as_mut()
            && host.id() == window_id
        {
            host.run_frame(gpu, |ui| main_window::ui(ui, state, core));
            return false;
        }

        if let Some((host, state)) = self.launcher.as_mut()
            && host.id() == window_id
        {
            let out = host.run_frame(gpu, |ui| launcher::ui(ui, state, core));
            return fit_height(host, out.content_height, launcher::MAX_HEIGHT);
        }

        if let Some(d) = self.dialogs.iter_mut().find(|d| d.host.id() == window_id) {
            let window = d.host.window.clone();
            let dialog = &mut d.dialog;
            let out = d.host.run_frame(gpu, |ui| {
                if dialog.ui(ui, core, &window) {
                    core.push(Command::CloseDialog { window: window.id() });
                }
            });
            if d.host.auto_height {
                return fit_height(&mut d.host, out.content_height, 900.0);
            }
        }
        false
    }

    /// 管理画面への外からのドラッグ＆ドロップ。
    fn on_drop(&mut self, event: crate::dragdrop::DropEvent) {
        use crate::dragdrop::DropEvent;
        let Some((host, state)) = self.main.as_mut() else { return };
        match event {
            DropEvent::Over { x, y, files } => {
                state.set_external_drag(screen_to_local(&host.window, x, y).map(|pos| (pos, files)));
                host.request_repaint();
            }
            DropEvent::Leave => {
                state.set_external_drag(None);
                host.request_repaint();
            }
            DropEvent::Drop { x, y, paths, text } => {
                state.set_external_drag(None);
                host.request_repaint();
                // ダイアログを開いている間は受け付けない（管理画面は操作できない状態）
                if !self.dialogs.is_empty() {
                    return;
                }
                let pos = screen_to_local(&host.window, x, y);
                let Some(folder) = state.drop_destination(pos, !paths.is_empty()) else { return };
                if !paths.is_empty() {
                    self.core.push(Command::AddPaths { paths, folder });
                } else if let Some(text) = text {
                    // ブラウザのアドレスバーなどからドラッグした URL を入れて登録画面を開く
                    self.core.push(Command::OpenEdit { existing: None, folder, initial: Some(text), owner: Owner::Main });
                }
            }
        }
    }

    /// 裏の処理（アイコンの取得など）の結果を反映する。
    fn process_icon_events(&mut self) {
        let mut changed = self.core.icons.poll(&mut self.core.store) | self.core.icons.apply_pending(&mut self.core.store);
        for (id, file) in std::mem::take(&mut self.core.icons.last_force_results) {
            let mut handled = false;
            for d in &mut self.dialogs {
                if d.dialog.on_favicon_result(&self.core, &id, file.as_deref()) {
                    handled = true;
                    d.host.request_repaint();
                }
            }
            // 「アイコンを再取得」で見つからなかったときは、理由を状態欄に出す
            if !handled && file.is_none() && self.core.store.find_shortcut(&id).is_some() {
                self.core.status(loc::t("Str.Main.IconNotInBrowser"));
                changed = true;
            }
        }
        if changed {
            self.request_repaint_all();
        }
    }
}

/// 中身に合わせてウィンドウの高さを変える（C# 版の SizeToContent="Height"）。
fn fit_height(host: &mut WindowHost, content_height: f32, max: f32) -> bool {
    if !host.auto_height || content_height <= 0.0 {
        return false;
    }
    let scale = host.window.scale_factor();
    let current = host.window.inner_size().to_logical::<f64>(scale);
    let wanted = (content_height as f64).clamp(80.0, max as f64).ceil();
    if (current.height - wanted).abs() > 1.0 {
        let _ = host.window.request_inner_size(LogicalSize::new(current.width, wanted));
        host.request_repaint();
        return true;
    }
    false
}

/// ウィンドウを表示して前面に出す（他のアプリを使っている途中にホットキーで呼ばれても前に出るように）。
fn show_and_activate(host: &mut WindowHost) {
    if dev_quiet() {
        // 確認用の起動では、利用者が打っている文字を奪わないよう、前面に出さず表示だけする
        match host.hwnd() {
            Some(hwnd) => platform::show_without_activating(hwnd),
            None => host.window.set_visible(true),
        }
    } else {
        host.window.set_visible(true);
        host.window.focus_window();
        if let Some(hwnd) = host.hwnd() {
            platform::force_foreground(hwnd);
        }
    }
    host.request_repaint();
}

/// 開発・確認用: 保存場所を環境変数で指定し、FAVORITESHORTCUT_DEV_QUIET=1 のときは
/// ウィンドウを前面に出さない（画面の確認中に、利用者が別のアプリで打っている文字を奪わないため）。
fn dev_quiet() -> bool {
    crate::paths::is_overridden_by_environment() && std::env::var("FAVORITESHORTCUT_DEV_QUIET").as_deref() == Ok("1")
}

/// グローバルホットキーを登録してよいか。開発・確認用に保存場所を環境変数で指定しているときは、
/// 普段使いのアプリの邪魔をしないよう、FAVORITESHORTCUT_DEV_HOTKEYS=1 のときだけ登録する。
fn hotkeys_allowed() -> bool {
    !crate::paths::is_overridden_by_environment() || std::env::var("FAVORITESHORTCUT_DEV_HOTKEYS").as_deref() == Ok("1")
}

fn zip_filter_name() -> String {
    // C# 版の "ZIP ファイル (*.zip)|*.zip" から、表示名の部分だけを使う
    loc::t("Str.Transfer.Filter").split('|').next().unwrap_or("ZIP").to_string()
}

pub fn fatal(text: &str) {
    crate::log::error(text);
    platform::message(None, text, &loc::t("Str.App.Name"), MsgIcon::Error);
}

// ------------------------------------------------------------ ウィンドウの作成

fn app_window_icon() -> Option<winit::window::Icon> {
    let (rgba, w, h) = crate::tray::app_icon_rgba(48)?;
    winit::window::Icon::from_rgba(rgba, w, h).ok()
}

/// 画面上の座標（物理ピクセル）を、ウィンドウ内の egui の座標に変える。
fn screen_to_local(window: &Window, x: i32, y: i32) -> Option<egui::Pos2> {
    let inner = window.inner_position().ok()?;
    let scale = window.scale_factor() as f32;
    Some(egui::pos2((x - inner.x) as f32 / scale, (y - inner.y) as f32 / scale))
}

fn create_main_window(el: &ActiveEventLoop, s: &AppSettings, dark: bool) -> Result<Arc<Window>, winit::error::OsError> {
    use winit::platform::windows::WindowAttributesExtWindows;
    let width = s.main_window_width.max(780.0);
    let height = s.main_window_height.max(460.0);
    let mut attrs = Window::default_attributes()
        .with_title(loc::t("Str.App.Name"))
        .with_inner_size(LogicalSize::new(width, height))
        .with_min_inner_size(LogicalSize::new(780.0, 460.0))
        .with_visible(false)
        .with_window_icon(app_window_icon())
        .with_theme(Some(if dark { winit::window::Theme::Dark } else { winit::window::Theme::Light }))
        // ドラッグ＆ドロップは自前で受け付ける（dragdrop.rs）
        .with_drag_and_drop(false);
    // 画面の中央に出す（C# 版の WindowStartupLocation="CenterScreen"）
    if let Some(monitor) = el.primary_monitor() {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let pos = monitor.position().to_logical::<f64>(scale);
        let x = pos.x + ((size.width - width) / 2.0).max(0.0);
        let y = pos.y + ((size.height - height) / 2.0).max(0.0);
        attrs = attrs.with_position(winit::dpi::LogicalPosition::new(x, y));
    }
    el.create_window(attrs).map(Arc::new)
}

fn create_launcher_window(el: &ActiveEventLoop) -> Result<Arc<Window>, winit::error::OsError> {
    use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
    let attrs = Window::default_attributes()
        .with_title(loc::t("Str.Launcher.Title"))
        .with_inner_size(LogicalSize::new(launcher::WIDTH as f64, 160.0))
        .with_decorations(false)
        .with_resizable(false)
        .with_visible(false)
        .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
        .with_skip_taskbar(true)
        .with_undecorated_shadow(true)
        .with_corner_preference(CornerPreference::Round)
        .with_drag_and_drop(false);
    el.create_window(attrs).map(Arc::new)
}

fn create_dialog_window(
    el: &ActiveEventLoop,
    title: &str,
    width: f32,
    height: Option<f32>,
    owner: Option<windows::Win32::Foundation::HWND>,
    dark: bool,
) -> Result<Arc<Window>, winit::error::OsError> {
    use winit::platform::windows::WindowAttributesExtWindows;
    let resizable = height.is_some();
    let mut attrs = Window::default_attributes()
        .with_title(title)
        .with_inner_size(LogicalSize::new(width as f64, height.unwrap_or(300.0) as f64))
        .with_min_inner_size(LogicalSize::new((width * 0.8) as f64, 200.0))
        .with_resizable(resizable)
        .with_maximized(false)
        .with_enabled_buttons(winit::window::WindowButtons::CLOSE)
        .with_visible(false)
        .with_window_icon(app_window_icon())
        .with_theme(Some(if dark { winit::window::Theme::Dark } else { winit::window::Theme::Light }));
    if let Some(owner) = owner {
        attrs = attrs.with_owner_window(owner.0 as isize).with_skip_taskbar(true);
    }
    // 位置は描いて大きさが決まってから、持ち主の中央に合わせる（present_dialog）
    el.create_window(attrs).map(Arc::new)
}

// ------------------------------------------------------------ イベントの処理

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if !self.started {
            self.started = true;
            self.start(el);
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let is_main = self.main.as_ref().is_some_and(|(h, _)| h.id() == window_id);
        let is_launcher = self.launcher.as_ref().is_some_and(|(h, _)| h.id() == window_id);
        let dialog_index = self.dialogs.iter().position(|d| d.host.id() == window_id);

        match &event {
            WindowEvent::CloseRequested => {
                if is_main {
                    // 閉じるボタンではアプリを終了せず、トレイに常駐させる（§67）
                    if self.core.settings.minimize_to_tray && self.tray.is_some() {
                        self.hide_main();
                    } else {
                        self.exit(el);
                    }
                } else if is_launcher {
                    self.hide_launcher();
                } else if dialog_index.is_some() {
                    if let Some(d) = self.dialogs.iter_mut().find(|d| d.host.id() == window_id) {
                        d.dialog.on_cancel(&mut self.core);
                    }
                    self.close_dialog(window_id);
                }
                self.process_commands(el);
                return;
            }
            WindowEvent::Focused(false) if is_launcher => {
                // 別のウィンドウをクリックしたら閉じる（§64）
                self.hide_launcher();
                return;
            }
            WindowEvent::ThemeChanged(_) => {
                if self.core.settings.theme == AppTheme::Auto {
                    let dark = windows_uses_dark_mode();
                    if dark != self.core.is_dark() {
                        self.apply_palette_for(dark);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let _ = self.redraw(window_id);
                self.process_commands(el);
                return;
            }
            _ => {}
        }

        let host = if is_main {
            self.main.as_mut().map(|(h, _)| h)
        } else if is_launcher {
            self.launcher.as_mut().map(|(h, _)| h)
        } else {
            dialog_index.and_then(|i| self.dialogs.get_mut(i)).map(|d| &mut d.host)
        };
        if let Some(host) = host
            && host.on_window_event(&event)
        {
            host.request_repaint();
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Platform(PlatformEvent::LauncherHotKey) => {
                // 隠した直後（ランチャー以外をクリックしてホットキーを押した場合など）は出し直さない
                let just_hidden = self.core.launcher_hidden_at.is_some_and(|t| t.elapsed() < Duration::from_millis(150));
                if !just_hidden {
                    self.toggle_launcher();
                }
            }
            UserEvent::Platform(PlatformEvent::ClipboardHotKey) => self.register_from_clipboard(el),
            UserEvent::Platform(PlatformEvent::ShowRequested) => self.show_main(el),
            UserEvent::Tray(action) => match action {
                TrayAction::OpenMain => self.show_main(el),
                TrayAction::ShowLauncher => self.show_launcher(),
                TrayAction::AddFromClipboard => self.register_from_clipboard(el),
                TrayAction::Settings => self.core.push(Command::OpenSettings),
                TrayAction::Exit => self.exit(el),
                TrayAction::OpenRecent(id) => {
                    if self.core.store.find_shortcut(&id).is_some() {
                        self.core.push(Command::Launch { ids: vec![id], from_main: false });
                    }
                }
            },
            UserEvent::Wake => {}
            UserEvent::Drop(event) => self.on_drop(event),
            UserEvent::LaunchFinished { id, result, reveal, from_main } => self.on_launch_finished(id, result, reveal, from_main),
        }
        self.process_icon_events();
        self.process_commands(el);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.exiting {
            return;
        }
        self.process_icon_events();
        self.process_rechecks();
        self.auto_backup.tick(&self.core.store, &self.core.settings);
        self.process_commands(el);

        // 次に起きる時刻（各ウィンドウの描き直し・アイコンの取り直し・自動バックアップ）
        let now = Instant::now();
        let mut next: Option<Instant> = Some(self.auto_backup.next_check());
        let mut consider = |t: Instant| next = Some(next.map_or(t, |n| n.min(t)));
        for (t, _) in &self.core.rechecks {
            consider(*t);
        }
        let mut hosts: Vec<&mut WindowHost> = Vec::new();
        if let Some((h, _)) = self.main.as_mut() {
            hosts.push(h);
        }
        if let Some((h, _)) = self.launcher.as_mut() {
            hosts.push(h);
        }
        for d in &mut self.dialogs {
            hosts.push(&mut d.host);
        }
        for h in hosts {
            if h.window.is_visible() != Some(true) {
                continue;
            }
            if let Some(t) = h.next_repaint {
                if t <= now {
                    h.window.request_redraw();
                } else {
                    consider(t);
                }
            }
        }
        el.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t.max(now)),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.tray = None;
        self.core.hotkeys = None;
    }
}

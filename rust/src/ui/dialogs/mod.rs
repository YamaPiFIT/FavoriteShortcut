//! ダイアログ（登録・編集、設定、お気に入りの取り込み、タグの管理など）。
//!
//! どのダイアログも別の OS ウィンドウとして開き、持ち主のウィンドウを操作できなくする（モーダル）。
//! 閉じるときは ui() が true を返し、結果（Outcome）をアプリ本体が受け取って反映する。

use std::path::PathBuf;
use std::sync::Arc;

use egui::{CornerRadius, Key, Modifiers, Ui};
use winit::window::{Window, WindowId};

use crate::app::Core;
use crate::loc;
use crate::platform::{self, MsgIcon};
use crate::transfer::{ExportManifest, ImportMode};
use crate::ui::style::{self, Palette};
use crate::ui::widgets;
use crate::util::culture_cmp;

mod bookmarks;
mod edit;
mod settings;
mod small;

/// 1 行入力のダイアログを何のために開いたか。
#[derive(Clone, Debug)]
pub enum TextPurpose {
    NewFolder {
        parent: Option<String>,
    },
    RenameFolder {
        id: String,
    },
    RenameTag {
        old: String,
    },
    /// 登録画面の「新規フォルダ...」から（作ったフォルダをその画面で選ぶ）
    NewFolderForDialog {
        parent: Option<String>,
        dialog: WindowId,
    },
}

/// ダイアログを閉じたときの結果。
pub enum Outcome {
    None,
    TextEntered { purpose: TextPurpose, value: String },
    MoveTo { ids: Vec<String>, folder: Option<String> },
    DeleteFolder { folder: String, delete_shortcuts: bool },
    Import { path: PathBuf, mode: ImportMode },
    BookmarksImported { imported: usize, skipped: usize },
}

pub enum Dialog {
    Edit(Box<edit::EditDialog>),
    Settings(Box<settings::SettingsDialog>),
    Bookmarks(Box<bookmarks::BookmarkDialog>),
    TagManager(small::TagManager),
    FolderPicker(small::FolderPicker),
    FolderDelete(small::FolderDelete),
    TextInput(small::TextInput),
    ImportMode(small::ImportModeDialog),
}

impl Dialog {
    pub fn edit(core: &mut Core, existing: Option<String>, folder: Option<String>, initial: Option<String>) -> Self {
        Dialog::Edit(Box::new(edit::EditDialog::new(core, existing, folder, initial)))
    }

    pub fn settings(core: &Core) -> Self {
        Dialog::Settings(Box::new(settings::SettingsDialog::new(core)))
    }

    pub fn bookmarks(core: &Core, folder: Option<String>) -> Self {
        Dialog::Bookmarks(Box::new(bookmarks::BookmarkDialog::new(core, folder)))
    }

    pub fn tag_manager(core: &Core) -> Self {
        Dialog::TagManager(small::TagManager::new(core))
    }

    pub fn folder_picker(ids: Vec<String>, current: Option<String>) -> Self {
        Dialog::FolderPicker(small::FolderPicker::new(ids, current))
    }

    pub fn folder_delete(core: &Core, folder: String) -> Self {
        Dialog::FolderDelete(small::FolderDelete::new(core, folder))
    }

    pub fn text_input(purpose: TextPurpose, title: String, label: String, initial: String) -> Self {
        Dialog::TextInput(small::TextInput::new(purpose, title, label, initial))
    }

    pub fn import_mode(path: PathBuf, manifest: ExportManifest) -> Self {
        Dialog::ImportMode(small::ImportModeDialog::new(path, manifest))
    }

    /// (ウィンドウのタイトル, 幅, 高さ)。高さが None のダイアログは中身に合わせる（サイズ変更不可）。
    pub fn window_spec(&self) -> (String, f32, Option<f32>) {
        match self {
            Dialog::Edit(d) => (d.title(), 620.0, None),
            Dialog::Settings(_) => (loc::t("Str.Settings.Title"), 620.0, Some(700.0)),
            Dialog::Bookmarks(_) => (loc::t("Str.Bookmark.Title"), 600.0, Some(640.0)),
            Dialog::TagManager(_) => (loc::t("Str.TagManager.Title"), 430.0, Some(500.0)),
            Dialog::FolderPicker(_) => (loc::t("Str.FolderPicker.Title"), 400.0, Some(440.0)),
            Dialog::FolderDelete(_) => (loc::t("Str.FolderDelete.Title"), 460.0, None),
            Dialog::TextInput(d) => (d.title(), 420.0, None),
            Dialog::ImportMode(_) => (loc::t("Str.ImportMode.Title"), 520.0, None),
        }
    }

    pub fn is_edit(&self) -> bool {
        matches!(self, Dialog::Edit(_))
    }

    /// 1 フレーム描く。閉じるなら true。
    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core, window: &Arc<Window>) -> bool {
        let p = core.palette;
        paint_background(ui, &p);
        match self {
            Dialog::Edit(d) => d.ui(ui, core, window),
            Dialog::Settings(d) => d.ui(ui, core, window),
            Dialog::Bookmarks(d) => d.ui(ui, core, window),
            Dialog::TagManager(d) => d.ui(ui, core, window),
            Dialog::FolderPicker(d) => d.ui(ui, core),
            Dialog::FolderDelete(d) => d.ui(ui, core),
            Dialog::TextInput(d) => d.ui(ui, core),
            Dialog::ImportMode(d) => d.ui(ui, core, window),
        }
    }

    /// 閉じるボタン（✕）で閉じられたとき。
    pub fn on_cancel(&mut self, core: &mut Core) {
        match self {
            Dialog::Edit(d) => d.cancel(core),
            Dialog::Settings(d) => d.cancel(core),
            _ => {}
        }
    }

    pub fn into_outcome(self) -> Outcome {
        match self {
            Dialog::Edit(_) | Dialog::Settings(_) | Dialog::TagManager(_) => Outcome::None,
            Dialog::Bookmarks(d) => d.into_outcome(),
            Dialog::FolderPicker(d) => d.into_outcome(),
            Dialog::FolderDelete(d) => d.into_outcome(),
            Dialog::TextInput(d) => d.into_outcome(),
            Dialog::ImportMode(d) => d.into_outcome(),
        }
    }

    /// タグの名前が変わったとき（タグの管理画面を作り直す）。
    pub fn on_tags_changed(&mut self, core: &Core) {
        if let Dialog::TagManager(d) = self {
            d.reload(core);
        }
    }

    /// 「新規フォルダ...」で作ったフォルダを選ぶ。
    pub fn on_folder_created(&mut self, core: &Core, id: String) {
        if let Dialog::Edit(d) = self {
            d.on_folder_created(core, id);
        }
    }

    /// 「ブラウザから取得」の結果。この画面が頼んだものなら true。
    pub fn on_favicon_result(&mut self, core: &Core, id: &str, file: Option<&str>) -> bool {
        match self {
            Dialog::Edit(d) => d.on_favicon_result(core, id, file),
            _ => false,
        }
    }
}

// ------------------------------------------------------------------ 共通の部品

/// ウィンドウ全体を背景色で塗る。
pub(crate) fn paint_background(ui: &Ui, p: &Palette) {
    let rect = ui.ctx().content_rect();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.window_background);
}

/// Win32 のウィンドウハンドル（メッセージボックスやファイル選択の持ち主）。
pub(crate) fn hwnd_of(window: &Window) -> Option<windows::Win32::Foundation::HWND> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(windows::Win32::Foundation::HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

pub(crate) fn message(window: &Window, text: &str, title: &str, icon: MsgIcon) {
    platform::message(hwnd_of(window), text, title, icon);
}

pub(crate) fn confirm(window: &Window, text: &str, title: &str, icon: MsgIcon) -> bool {
    platform::confirm(hwnd_of(window), text, title, icon)
}

/// Esc が押されたか（押されていれば使ったことにする）。
pub(crate) fn take_escape(ui: &Ui) -> bool {
    ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
}

/// Enter が押されたか（押されていれば使ったことにする）。
pub(crate) fn take_enter(ui: &Ui) -> bool {
    ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
}

/// 見出し（SectionHeaderText）。下に 6 の余白。
pub(crate) fn header(ui: &mut Ui, p: &Palette, text: &str) {
    widgets::section_header(ui, p, text);
    ui.add_space(6.0 - ui.spacing().item_spacing.y);
}

/// フォルダの選択肢（「未分類」+ 全フォルダをフルパス順）。
pub(crate) fn folder_choices(core: &Core) -> Vec<(Option<String>, String)> {
    let mut folders: Vec<(String, String)> = core.store.folders().map(|f| (f.id.clone(), f.full_path.clone())).collect();
    folders.sort_by(|a, b| culture_cmp(&a.1, &b.1));
    let mut choices = vec![(None, loc::t("Str.Folder.UncategorizedChoice"))];
    choices.extend(folders.into_iter().map(|(id, path)| (Some(id), path)));
    choices
}

/// フォルダを選ぶコンボボックス。
pub(crate) fn folder_combo(
    ui: &mut Ui,
    id_salt: &str,
    choices: &[(Option<String>, String)],
    selected: &mut Option<String>,
    width: f32,
    enabled: bool,
) {
    let current = choices
        .iter()
        .find(|(id, _)| id == selected)
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| choices.first().map(|c| c.1.clone()).unwrap_or_default());
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(id_salt).selected_text(current).width(width).height(320.0).show_ui(ui, |ui| {
            for (id, label) in choices {
                ui.selectable_value(selected, id.clone(), label);
            }
        });
    });
}

/// 枠付きのカード（設定画面などの Card スタイル）。
pub(crate) fn card(ui: &mut Ui, p: &Palette, add_contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        });
}

/// 枠付きの箱（余白と角丸を指定できる）。
pub(crate) fn card_like(ui: &mut Ui, p: &Palette, padding: i8, radius: u8, add_contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(radius))
        .inner_margin(egui::Margin::same(padding))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        });
}

/// rfd のファイル選択用の絞り込み（C# の "名前|*.png;*.jpg" 形式から作る）。
pub(crate) fn filter_from_csharp(filter: &str) -> (String, Vec<String>) {
    let mut parts = filter.split('|');
    let name = parts.next().unwrap_or_default().to_string();
    let patterns = parts.next().unwrap_or("*.*");
    let exts = patterns.split(';').map(|p| p.trim().trim_start_matches("*.").to_string()).filter(|e| !e.is_empty()).collect();
    (name, exts)
}

/// 補足の文字（左に余白をつけたもの。チェックボックスの説明など）。
pub(crate) fn indented_caption(ui: &mut Ui, p: &Palette, indent: f32, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.vertical(|ui| {
            widgets::caption(ui, p, text);
        });
    });
}

/// 赤い文字（入力エラー）。
pub(crate) fn error_text(ui: &mut Ui, p: &Palette, text: &str, size: f32) {
    ui.add(egui::Label::new(widgets::rich(text, style::regular(size), p.danger)).wrap());
}

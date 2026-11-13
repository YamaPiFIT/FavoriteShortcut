//! ショートカットの新規登録・編集（C# 版 ShortcutEditWindow と同じ構成、§8）。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use egui::{Align, CornerRadius, Layout, Rect, Ui, vec2};
use winit::window::Window;

use super::{
    TextPurpose, card_like, confirm, error_text, filter_from_csharp, folder_choices, folder_combo, header, message, take_enter, take_escape,
};
use crate::app::{Command, Core};
use crate::loc;
use crate::model::{Shortcut, TargetType};
use crate::platform::MsgIcon;
use crate::target;
use crate::ui::style::{self, Palette};
use crate::ui::tag_input::TagInput;
use crate::ui::widgets::{self, ButtonKind};
use crate::util::eq_ignore_case;

/// URL / パスの入力が止まってから確認するまでの時間（C# 版と同じ 250 ミリ秒）
const TARGET_CHECK_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Target,
    Title,
    TitleSelectAll,
}

struct TargetCheck {
    kind: TargetType,
    normalized: String,
    exists: bool,
}

fn check_target(raw: &str) -> TargetCheck {
    let kind = target::detect(raw);
    let normalized = target::normalize(raw, kind);
    let exists = target::exists(&normalized, kind);
    TargetCheck { kind, normalized, exists }
}

pub struct EditDialog {
    /// 編集する既存項目の ID（新規登録なら None）
    original: Option<String>,
    target: String,
    title: String,
    note: String,
    tags: TagInput,
    folder_choices: Vec<(Option<String>, String)>,
    folder: Option<String>,
    /// ダイアログ上で選択中のアイコン（icons/ 配下の相対ファイル名）。None なら自動
    icon_path: Option<String>,
    /// この画面で新しく取り込んだカスタムアイコン。キャンセル時に後始末する
    temporary_icons: Vec<String>,

    detected: TargetType,
    exists: bool,
    info: String,
    target_changed_at: Option<Instant>,
    /// URL / パスの確認の世代。裏で確かめている間に入力が変わったら、古い結果は使わない
    check_generation: u64,
    check_rx: Option<Receiver<(u64, TargetCheck)>>,

    /// 「ブラウザから取得」中の問い合わせ ID
    fetching: Option<String>,
    pending_message: Option<(String, String, MsgIcon)>,
    error: Option<String>,
    focus: Option<Field>,
}

impl EditDialog {
    pub fn new(core: &mut Core, existing: Option<String>, default_folder: Option<String>, initial: Option<String>) -> Self {
        let original = existing.and_then(|id| core.store.find_shortcut(&id).cloned());
        let mut tags = TagInput::new("edit");
        tags.set_available(core.store.all_tag_names());
        let folder = original.as_ref().map_or(default_folder, |o| o.folder_id.clone());
        let choices = folder_choices(core);
        let folder = if choices.iter().any(|(id, _)| *id == folder) { folder } else { None };

        let mut dialog = Self {
            original: original.as_ref().map(|o| o.id.clone()),
            target: String::new(),
            title: String::new(),
            note: String::new(),
            tags,
            folder_choices: choices,
            folder,
            icon_path: None,
            temporary_icons: Vec::new(),
            detected: TargetType::Unknown,
            exists: false,
            info: String::new(),
            target_changed_at: None,
            check_generation: 0,
            check_rx: None,
            fetching: None,
            pending_message: None,
            error: None,
            focus: None,
        };
        if let Some(o) = &original {
            dialog.target = o.target.clone();
            dialog.title = o.title.clone();
            dialog.note = o.note.clone().unwrap_or_default();
            dialog.tags.set_tags(&o.tags);
            dialog.icon_path = o.icon_path.clone();
        } else if let Some(t) = initial.filter(|t| !t.trim().is_empty()) {
            dialog.target = t.trim().to_string();
        }
        dialog.update_target_info();

        dialog.focus = Some(if dialog.target.is_empty() {
            Field::Target
        } else if dialog.title.is_empty() {
            Field::Title
        } else {
            Field::TitleSelectAll
        });
        dialog
    }

    pub fn title(&self) -> String {
        loc::t(if self.original.is_some() { "Str.Edit.TitleEdit" } else { "Str.Edit.TitleNew" })
    }

    // ------------------------------------------------------- URL / パスの判定

    fn update_target_info(&mut self) {
        let raw = self.target.trim().to_string();
        self.check_generation += 1;
        self.check_rx = None;

        if raw.is_empty() {
            self.detected = TargetType::Unknown;
            self.exists = false;
            self.info = loc::t("Str.Edit.TargetHint");
            return;
        }

        if target::is_network_path(&target::expand(&target::normalize(&raw, TargetType::Unknown))) {
            // 共有フォルダはサーバーの応答を待つことがあるので、画面を止めずに裏で確かめる
            self.info = loc::t("Str.Edit.CheckingPath");
            let (tx, rx) = mpsc::channel();
            let generation = self.check_generation;
            let raw2 = raw.clone();
            let spawned = std::thread::Builder::new().name("TargetCheck".into()).spawn(move || {
                let _ = tx.send((generation, check_target(&raw2)));
            });
            if spawned.is_ok() {
                self.check_rx = Some(rx);
                return;
            }
            crate::log::warn(&format!("パスを確認できませんでした: {raw}"));
            self.apply_check(&raw, TargetCheck { kind: TargetType::Unknown, normalized: raw.clone(), exists: true });
            return;
        }
        let result = check_target(&raw);
        self.apply_check(&raw, result);
    }

    fn apply_check(&mut self, raw: &str, result: TargetCheck) {
        self.detected = result.kind;
        self.exists = result.exists;
        let mut info = loc::tf("Str.Edit.TypeInfo", &[&self.detected.display_name()]);
        if self.detected == TargetType::Web && result.normalized != raw {
            info += &loc::tf("Str.Edit.SavedAs", &[&result.normalized]);
        }
        if !result.exists {
            info += &loc::t("Str.Edit.PathMissing");
        }
        self.info = info;
        // タイトル未入力なら、対象から推測した名前を入れておく
        if self.title.trim().is_empty() {
            self.title = target::suggest_title(&result.normalized, self.detected);
        }
    }

    fn poll_check(&mut self, ui: &Ui) {
        if let Some(t) = self.target_changed_at {
            if t.elapsed() >= TARGET_CHECK_DELAY {
                self.target_changed_at = None;
                self.update_target_info();
            } else {
                ui.ctx().request_repaint_after(TARGET_CHECK_DELAY - t.elapsed());
            }
        }
        if let Some(rx) = &self.check_rx {
            match rx.try_recv() {
                Ok((generation, result)) => {
                    self.check_rx = None;
                    if generation == self.check_generation {
                        let raw = self.target.trim().to_string();
                        self.apply_check(&raw, result);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => self.check_rx = None,
            }
        }
    }

    // ------------------------------------------------------------- フォルダ

    pub fn on_folder_created(&mut self, core: &Core, id: String) {
        self.folder_choices = folder_choices(core);
        self.folder = Some(id);
    }

    // ------------------------------------------------------------- アイコン

    fn icon_source_text(&self) -> String {
        match self.icon_path.as_deref() {
            None => loc::t(match self.detected {
                TargetType::Web => "Str.Edit.IconAutoWeb",
                TargetType::Folder => "Str.Edit.IconAutoFolder",
                TargetType::File | TargetType::Application => "Str.Edit.IconAutoFile",
                TargetType::Unknown => "Str.Edit.IconAutoDefault",
            }),
            Some(p) if p.starts_with("custom_") => loc::t("Str.Edit.IconCustom"),
            Some(_) => loc::t("Str.Edit.IconCached"),
        }
    }

    fn choose_icon(&mut self, core: &mut Core, window: &Arc<Window>) {
        let (name, exts) = filter_from_csharp(&loc::t("Str.Edit.ImageFilter"));
        let Some(path) = rfd::FileDialog::new()
            .set_title(loc::t("Str.Edit.ChooseImageTitle"))
            .add_filter(name, &exts)
            .set_parent(window.as_ref())
            .pick_file()
        else {
            return;
        };
        match core.icons.import_custom_icon(&path) {
            Some(saved) => {
                self.icon_path = Some(saved.clone());
                self.temporary_icons.push(saved);
            }
            None => message(window, &loc::t("Str.Edit.ImageUnreadable"), &loc::t("Str.Edit.Icon"), MsgIcon::Warning),
        }
    }

    fn fetch_favicon(&mut self, core: &mut Core, window: &Arc<Window>) {
        let target = target::normalize(self.target.trim(), TargetType::Web);
        if target::try_get_host(&target).is_none() {
            message(window, &loc::t("Str.Edit.NoHost"), &loc::t("Str.Edit.FetchFavicon"), MsgIcon::Warning);
            return;
        }
        let probe = Shortcut { target, target_type: TargetType::Web, ..Shortcut::new() };
        let use_cache = core.settings.use_browser_icon_cache;
        if core.icons.request_favicon(&probe, true, use_cache, core.is_dark()) {
            self.fetching = Some(probe.id);
        } else {
            message(window, &favicon_failed_text(use_cache), &loc::t("Str.Edit.FetchFavicon"), MsgIcon::Info);
        }
    }

    pub fn on_favicon_result(&mut self, core: &Core, id: &str, file: Option<&str>) -> bool {
        if self.fetching.as_deref() != Some(id) {
            return false;
        }
        self.fetching = None;
        match file {
            Some(name) => self.icon_path = Some(name.to_string()),
            None => {
                // アイコンはブラウザが保存しているものだけを使うので、見つからない理由を案内する
                self.pending_message =
                    Some((favicon_failed_text(core.settings.use_browser_icon_cache), loc::t("Str.Edit.FetchFavicon"), MsgIcon::Info));
            }
        }
        true
    }

    // ---------------------------------------------------------------- 参照

    fn browse_file(&mut self, window: &Arc<Window>) {
        let (name, exts) = filter_from_csharp(&loc::t("Str.Edit.AllFilesFilter"));
        let exts: Vec<String> = exts.into_iter().map(|e| if e == "*.*" || e == "*" { "*".into() } else { e }).collect();
        if let Some(path) = rfd::FileDialog::new()
            .set_title(loc::t("Str.Edit.ChooseFileTitle"))
            .add_filter(name, &exts)
            .set_parent(window.as_ref())
            .pick_file()
        {
            self.target = path.to_string_lossy().into_owned();
            self.target_changed_at = None;
            self.update_target_info();
        }
    }

    fn browse_folder(&mut self, window: &Arc<Window>) {
        if let Some(path) = rfd::FileDialog::new().set_title(loc::t("Str.Edit.ChooseFolderTitle")).set_parent(window.as_ref()).pick_folder()
        {
            self.target = path.to_string_lossy().into_owned();
            self.target_changed_at = None;
            self.update_target_info();
        }
    }

    // ---------------------------------------------------------------- 保存

    /// 保存できたら true（閉じる）。
    fn save(&mut self, core: &mut Core, window: &Arc<Window>) -> bool {
        // 入力途中のタグを取りこぼさない
        self.tags.commit_pending();

        let raw = self.target.trim().to_string();
        if raw.is_empty() {
            self.error = Some(loc::t("Str.Edit.TargetRequired"));
            self.focus = Some(Field::Target);
            return false;
        }
        self.detected = target::detect(&raw);
        let target = target::normalize(&raw, self.detected);

        let mut title = self.title.trim().to_string();
        if title.is_empty() {
            title = target::suggest_title(&target, self.detected);
        }
        if title.is_empty() {
            self.error = Some(loc::t("Str.Edit.TitleRequired"));
            self.focus = Some(Field::Title);
            return false;
        }

        // 同じ URL / パスが既に登録されている場合に確認する（§38: 重複自体は許可する）
        if let Some(dup) =
            core.store.shortcuts().iter().find(|s| Some(&s.id) != self.original.as_ref() && eq_ignore_case(&s.target, &target))
        {
            let text = loc::tf("Str.Edit.Duplicate", &[&dup.display_title(), &dup.folder_path_text]);
            if !confirm(window, &text, &loc::t("Str.Edit.DuplicateTitle"), MsgIcon::Question) {
                return false;
            }
        }

        let note = self.note.trim().to_string();
        let note = if note.is_empty() { None } else { Some(note) };
        let result = match &self.original {
            None => {
                let item = Shortcut {
                    title,
                    target,
                    target_type: self.detected,
                    folder_id: self.folder.clone(),
                    icon_path: self.icon_path.clone(),
                    note,
                    tags: self.tags.tags().to_vec(),
                    ..Shortcut::new()
                };
                let probe = item.clone();
                core.store.add_shortcut(item).map(|_| {
                    if probe.target_type == TargetType::Web && probe.icon_path.is_none() {
                        let use_cache = core.settings.use_browser_icon_cache;
                        let dark = core.is_dark();
                        core.icons.request_favicon(&probe, false, use_cache, dark);
                    }
                })
            }
            Some(id) => match core.store.find_shortcut(id).cloned() {
                // 編集している間に削除された場合は、新しく登録し直す
                None => {
                    let item = Shortcut {
                        title,
                        target,
                        target_type: self.detected,
                        folder_id: self.folder.clone(),
                        icon_path: self.icon_path.clone(),
                        note,
                        tags: self.tags.tags().to_vec(),
                        ..Shortcut::new()
                    };
                    core.store.add_shortcut(item).map(|_| ())
                }
                Some(original) => {
                    // アイコンを差し替えたなら、使われなくなったカスタム画像を片付ける
                    if original.icon_path != self.icon_path {
                        core.icons.delete_custom_icon(original.icon_path.as_deref());
                    }
                    let mut edited = original.clone();
                    edited.title = title;
                    edited.target = target;
                    edited.target_type = self.detected;
                    edited.folder_id = self.folder.clone();
                    edited.icon_path = self.icon_path.clone();
                    edited.note = note;
                    edited.tags = self.tags.tags().to_vec();
                    core.store.update_shortcut(&edited)
                }
            },
        };
        match result {
            Ok(()) => {
                // 保存できたので、この画面で取り込んだ画像は後始末の対象から外す
                self.temporary_icons.clear();
                core.push(Command::RefreshTray);
                true
            }
            Err(e) => {
                crate::log::error_err("ショートカットの保存に失敗しました。", &e);
                self.error = Some(loc::tf("Str.Edit.SaveFailed", &[&e]));
                false
            }
        }
    }

    fn delete(&mut self, core: &mut Core, window: &Arc<Window>) -> bool {
        let Some(id) = self.original.clone() else { return false };
        let Some(item) = core.store.find_shortcut(&id).cloned() else { return true };
        let text = format!("{}\n\n{}", loc::tf("Str.Common.ConfirmDeleteOne", &[&item.display_title()]), loc::t("Str.Common.CannotUndo"));
        if !confirm(window, &text, &loc::t("Str.Common.ConfirmDeleteTitle"), MsgIcon::Warning) {
            return false;
        }
        core.icons.delete_custom_icon(item.icon_path.as_deref());
        if let Err(e) = core.store.delete_shortcut(&id) {
            crate::log::warn_err("削除に失敗しました。", &e);
        }
        self.cancel(core);
        core.push(Command::RefreshTray);
        true
    }

    /// キャンセルで閉じるとき、この画面で取り込んだ画像ファイルは不要なので削除する。
    pub fn cancel(&mut self, core: &mut Core) {
        for icon in std::mem::take(&mut self.temporary_icons) {
            core.icons.delete_custom_icon(Some(&icon));
        }
    }

    // ---------------------------------------------------------------- 画面

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core, window: &Arc<Window>) -> bool {
        let p = core.palette;
        if let Some((text, title, icon)) = self.pending_message.take() {
            message(window, &text, &title, icon);
        }
        self.poll_check(ui);

        let mut close = false;
        let mut consumed_enter = false;
        let mut consumed_escape = false;
        let mut note_focused = false;

        egui::Frame::new().inner_margin(egui::Margin { left: 22, right: 22, top: 20, bottom: 18 }).show(ui, |ui| {
            let width = ui.ctx().content_rect().width() - 44.0;
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);

            // ---- URL / パス
            header(ui, &p, &loc::t("Str.Edit.Target"));
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::button(ui, &p, ButtonKind::Secondary, &loc::t("Str.Edit.BrowseFolder")).clicked() {
                        self.browse_folder(window);
                    }
                    if widgets::button(ui, &p, ButtonKind::Secondary, &loc::t("Str.Edit.BrowseFile")).clicked() {
                        self.browse_file(window);
                    }
                    ui.add_space(2.0);
                    let id = egui::Id::unique("edit-target");
                    let before = self.target.clone();
                    let response = widgets::text_field(ui, &mut self.target, id, ui.available_width());
                    if self.target != before {
                        self.target_changed_at = Some(Instant::now());
                        ui.ctx().request_repaint_after(TARGET_CHECK_DELAY);
                    }
                    if self.focus == Some(Field::Target) {
                        self.focus = None;
                        response.request_focus();
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                ui.vertical(|ui| {
                    widgets::caption(ui, &p, &self.info);
                });
            });

            // ---- タイトル
            ui.add_space(16.0);
            header(ui, &p, &loc::t("Str.Edit.TitleLabel"));
            let title_id = egui::Id::unique("edit-title");
            let response = widgets::text_field(ui, &mut self.title, title_id, width);
            match self.focus {
                Some(Field::Title) => {
                    self.focus = None;
                    response.request_focus();
                }
                Some(Field::TitleSelectAll) => {
                    self.focus = None;
                    response.request_focus();
                    select_all(ui, title_id, &self.title);
                }
                _ => {}
            }

            // ---- タグ
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                widgets::section_header(ui, &p, &loc::t("Str.Tag.Section"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(widgets::rich(&loc::t("Str.Edit.TagHint"), style::regular(style::CAPTION), p.text_secondary));
                });
            });
            ui.add_space(6.0);
            let tag_response = self.tags.show(ui, &p);
            consumed_enter |= tag_response.consumed_enter;
            consumed_escape |= tag_response.consumed_escape;

            // ---- フォルダ
            ui.add_space(16.0);
            header(ui, &p, &loc::t("Str.Edit.Folder"));
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button(ui, &p, ButtonKind::Secondary, &loc::t("Str.Edit.NewFolder")).clicked() {
                        core.push(Command::OpenTextInput {
                            purpose: TextPurpose::NewFolderForDialog { parent: self.folder.clone(), dialog: window.id() },
                            title: loc::t("Str.Folder.NewTitle"),
                            label: loc::t("Str.Folder.NameLabel"),
                            initial: String::new(),
                        });
                    }
                    let w = ui.available_width();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        folder_combo(ui, "edit-folder", &self.folder_choices, &mut self.folder, w - 8.0, true);
                    });
                });
            });

            // ---- アイコン
            ui.add_space(16.0);
            header(ui, &p, &loc::t("Str.Edit.Icon"));
            card_like(ui, &p, 12, 8, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(52.0, 52.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(8), p.sidebar_background);
                    let preview_target = target::normalize(self.target.trim(), self.detected);
                    let icon = core.icons.resolve_preview(self.detected, &preview_target, self.icon_path.as_deref(), self.exists);
                    widgets::paint_icon(ui, Rect::from_center_size(rect.center(), vec2(36.0, 36.0)), &icon);
                    ui.add_space(14.0 - 8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        widgets::caption(ui, &p, &self.icon_source_text());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if widgets::button(ui, &p, ButtonKind::Secondary, &loc::t("Str.Edit.ChooseImage")).clicked() {
                                self.choose_icon(core, window);
                            }
                            let fetching = self.fetching.is_some();
                            let label = loc::t(if fetching { "Str.Edit.Fetching" } else { "Str.Edit.FetchFavicon" });
                            let enabled = self.detected == TargetType::Web && !fetching;
                            if widgets::button_ex(ui, &p, ButtonKind::Secondary, &label, enabled, None).clicked() {
                                self.fetch_favicon(core, window);
                            }
                            if widgets::button(ui, &p, ButtonKind::Ghost, &loc::t("Str.Common.ResetDefault")).clicked() {
                                self.icon_path = None;
                            }
                        });
                    });
                });
            });

            // ---- メモ（検索対象にもなる）
            ui.add_space(16.0);
            header(ui, &p, &loc::t("Str.Edit.Note"));
            let note = widgets::framed_text_edit_inset(
                ui,
                &p,
                vec2(width, 60.0),
                vec2(9.0, 6.0),
                egui::TextEdit::multiline(&mut self.note)
                    .id(egui::Id::unique("edit-note"))
                    .font(style::regular(style::BODY))
                    .margin(vec2(8.0, 6.0)),
            );
            note_focused = note.has_focus();

            // ---- エラー
            if let Some(error) = &self.error {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    error_text(ui, &p, error, style::BODY);
                });
            }

            // ---- ボタン
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                if self.original.is_some() {
                    let galley_color = p.danger;
                    let r = ghost_danger(ui, &p, &loc::t("Str.Common.Delete"), galley_color);
                    if r.clicked() && self.delete(core, window) {
                        close = true;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let save_text = loc::t(if self.original.is_some() { "Str.Common.Save" } else { "Str.Edit.Add" });
                    if widgets::button_ex(ui, &p, ButtonKind::Primary, &save_text, true, Some(100.0)).clicked() && self.save(core, window) {
                        close = true;
                    }
                    if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Cancel"), true, Some(100.0)).clicked() {
                        self.cancel(core);
                        close = true;
                    }
                });
            });
        });

        if close {
            return true;
        }
        // Esc でキャンセル、Enter で保存（メモ欄の Enter は改行）
        if !consumed_escape && take_escape(ui) {
            self.cancel(core);
            return true;
        }
        if !consumed_enter && !note_focused && take_enter(ui) && self.save(core, window) {
            return true;
        }
        false
    }
}

fn favicon_failed_text(use_cache: bool) -> String {
    loc::t(if use_cache { "Str.Edit.FaviconFailed" } else { "Str.Edit.BrowserIconCacheOff" })
}

/// 赤い文字の控えめなボタン（「削除」）。
fn ghost_danger(ui: &mut Ui, p: &Palette, text: &str, color: egui::Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), style::regular(style::BODY), color);
    let size = vec2(galley.size().x + 16.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), p.surface_hover);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// 入力欄の文字をすべて選択する。
pub(crate) fn select_all(ui: &Ui, id: egui::Id, text: &str) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let range = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
    state.cursor.set_char_range(Some(range));
    state.store(ui.ctx(), id);
}

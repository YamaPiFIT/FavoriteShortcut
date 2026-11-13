//! 小さなダイアログ（タグの管理・移動先フォルダの選択・フォルダの削除・1 行入力・インポート方法）。

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use winit::window::Window;

use super::{Outcome, TextPurpose, card_like, confirm, error_text, header, take_enter, take_escape};
use crate::app::{Command, Core};
use crate::loc;
use crate::platform::MsgIcon;
use crate::transfer::{ExportManifest, ImportMode};
use crate::ui::dialogs::edit::select_all;
use crate::ui::style::{self, Palette};
use crate::ui::widgets::{self, ButtonKind};
use crate::util::contains_ignore_case;

// =================================================================== タグの管理

/// タグの一覧・名前変更・削除。名前変更は使用中の全ショートカットに反映される。
pub struct TagManager {
    all: Vec<(String, usize)>,
    filter: String,
    selected: Option<String>,
}

impl TagManager {
    pub fn new(core: &Core) -> Self {
        let mut d = Self { all: Vec::new(), filter: String::new(), selected: None };
        d.reload(core);
        d
    }

    pub fn reload(&mut self, core: &Core) {
        self.all = core.store.tag_usage();
        if let Some(s) = &self.selected
            && !self.all.iter().any(|(n, _)| n == s)
        {
            self.selected = None;
        }
    }

    fn rename(&mut self, core: &mut Core) {
        let Some(name) = self.selected.clone() else { return };
        core.push(Command::OpenTextInput {
            purpose: TextPurpose::RenameTag { old: name.clone() },
            title: loc::t("Str.TagManager.RenameTitle"),
            label: loc::tf("Str.TagManager.RenamePrompt", &[&name]),
            initial: name,
        });
    }

    fn delete(&mut self, core: &mut Core, window: &Arc<Window>) {
        let Some(name) = self.selected.clone() else { return };
        let count = self.all.iter().find(|(n, _)| *n == name).map(|(_, c)| *c).unwrap_or(0);
        if !confirm(
            window,
            &loc::tf("Str.TagManager.DeleteConfirm", &[&name, &count]),
            &loc::t("Str.TagManager.DeleteTitle"),
            MsgIcon::Warning,
        ) {
            return;
        }
        if let Err(e) = core.store.delete_tag(&name) {
            crate::log::warn_err("タグを削除できませんでした。", &e);
        }
        self.reload(core);
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core, window: &Arc<Window>) -> bool {
        let p = core.palette;
        let mut close = false;
        egui::Panel::bottom("tags-buttons")
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 18, top: 14, bottom: 18 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Rename"), true, Some(92.0)).clicked() {
                        self.rename(core);
                    }
                    if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Delete"), true, Some(92.0)).clicked() {
                        self.delete(core, window);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button_ex(ui, &p, ButtonKind::Primary, &loc::t("Str.Common.Close"), true, Some(92.0)).clicked() {
                            close = true;
                        }
                    });
                });
            });

        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 18, top: 18, bottom: 0 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                header(ui, &p, &loc::t("Str.TagManager.Heading"));
                let w = ui.available_width();
                widgets::framed_text_edit(
                    ui,
                    &p,
                    vec2(w, 32.0),
                    egui::TextEdit::singleline(&mut self.filter)
                        .id(egui::Id::unique("tag-filter"))
                        .font(style::regular(style::BODY))
                        .margin(vec2(10.0, 6.0))
                        .hint_text(egui::RichText::new(loc::t("Str.TagManager.Filter")).color(p.text_disabled))
                        .align(egui::Align2::LEFT_CENTER),
                );
                ui.add_space(10.0);

                let filter = self.filter.trim().to_string();
                let rows: Vec<(String, usize)> =
                    self.all.iter().filter(|(n, _)| filter.is_empty() || contains_ignore_case(n, &filter)).cloned().collect();
                let height = ui.available_height();
                let mut double_clicked = false;
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_height((height - 10.0).max(40.0));
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (name, count) in &rows {
                                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                                let selected = self.selected.as_deref() == Some(name.as_str());
                                let fill = if selected {
                                    p.accent_soft
                                } else if response.hovered() {
                                    p.surface_hover
                                } else {
                                    Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
                                let inner = rect.shrink2(vec2(12.0, 0.0));
                                ui.painter().text(
                                    pos2(inner.left(), inner.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    name,
                                    style::regular(style::BODY),
                                    p.text_primary,
                                );
                                ui.painter().text(
                                    pos2(inner.right(), inner.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    loc::tf("Str.Common.CountItems", &[count]),
                                    style::regular(style::CAPTION),
                                    p.text_secondary,
                                );
                                if response.clicked() || response.secondary_clicked() {
                                    self.selected = Some(name.clone());
                                }
                                if response.double_clicked() {
                                    self.selected = Some(name.clone());
                                    double_clicked = true;
                                }
                            }
                        });
                    });
                if double_clicked {
                    self.rename(core);
                }
            });
        });

        close || take_escape(ui)
    }
}

// ============================================================ 移動先フォルダの選択

pub struct FolderPicker {
    ids: Vec<String>,
    selected: Option<String>,
    done: bool,
}

impl FolderPicker {
    pub fn new(ids: Vec<String>, current: Option<String>) -> Self {
        Self { ids, selected: current, done: false }
    }

    pub fn into_outcome(self) -> Outcome {
        if self.done { Outcome::MoveTo { ids: self.ids, folder: self.selected } } else { Outcome::None }
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) -> bool {
        let p = core.palette;
        let mut close = false;
        egui::Panel::bottom("picker-buttons")
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 16, bottom: 16 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button_ex(ui, &p, ButtonKind::Primary, &loc::t("Str.FolderPicker.Move"), true, Some(92.0)).clicked() {
                            self.done = true;
                            close = true;
                        }
                        if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Cancel"), true, Some(92.0)).clicked() {
                            close = true;
                        }
                    });
                });
            });

        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 16, bottom: 0 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                header(ui, &p, &loc::t("Str.FolderPicker.Prompt"));
                let height = ui.available_height();
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_height((height - 14.0).max(40.0));
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let mut chosen: Option<(Option<String>, bool)> = None;
                            if let Some(c) =
                                picker_row(ui, &p, "◇", &loc::t("Str.Folder.UncategorizedChoice"), true, 0, self.selected.is_none())
                            {
                                chosen = Some((None, c));
                            }
                            let roots = core.store.root_folders().to_vec();
                            for id in roots {
                                self.subtree(ui, core, &p, &id, 0, &mut chosen);
                            }
                            if let Some((folder, double)) = chosen {
                                self.selected = folder;
                                if double {
                                    self.done = true;
                                    close = true;
                                }
                            }
                        });
                    });
            });
        });

        if close {
            return true;
        }
        if take_enter(ui) {
            self.done = true;
            return true;
        }
        take_escape(ui)
    }

    fn subtree(&self, ui: &mut Ui, core: &Core, p: &Palette, id: &str, depth: usize, chosen: &mut Option<(Option<String>, bool)>) {
        let Some(folder) = core.store.folder(id) else { return };
        let selected = self.selected.as_deref() == Some(id);
        if let Some(double) = picker_row(ui, p, "📁", &folder.name, false, depth + 1, selected) {
            *chosen = Some((Some(id.to_string()), double));
        }
        for child in folder.children.clone() {
            self.subtree(ui, core, p, &child, depth + 1, chosen);
        }
    }
}

/// ツリーの 1 行。クリックされたら Some(ダブルクリックか)。
fn picker_row(ui: &mut Ui, p: &Palette, glyph: &str, name: &str, bold: bool, depth: usize, selected: bool) -> Option<bool> {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let fill = if selected {
        p.accent_soft
    } else if response.hovered() {
        p.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    let x = rect.left() + 6.0 + 16.0 * depth.saturating_sub(1) as f32 + if depth > 0 { 16.0 } else { 0.0 };
    ui.painter().text(pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, glyph, style::regular(13.0), p.text_primary);
    let font = if bold { style::bold(style::BODY) } else { style::regular(style::BODY) };
    let max_w = rect.right() - (x + 22.0) - 4.0;
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), font, p.text_primary);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(10.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(x + 22.0, rect.center().y - galley.size().y / 2.0), galley, p.text_primary);
    if response.double_clicked() {
        Some(true)
    } else if response.clicked() {
        Some(false)
    } else {
        None
    }
}

// ================================================================ フォルダの削除

/// フォルダ削除の確認（§37）。中身の扱いを明示的に選ばせ、うっかりショートカットを失わないようにする。
pub struct FolderDelete {
    folder: String,
    name: String,
    detail: String,
    shortcut_count: usize,
    delete_shortcuts: bool,
    done: bool,
}

impl FolderDelete {
    pub fn new(core: &Core, folder: String) -> Self {
        let name = core.store.folder(&folder).map(|f| f.name.clone()).unwrap_or_default();
        let descendants = core.store.collect_folder_and_descendants(&folder);
        let shortcut_count =
            core.store.shortcuts().iter().filter(|s| s.folder_id.as_ref().is_some_and(|f| descendants.contains(f))).count();
        let sub_count = descendants.len().saturating_sub(1);
        let mut parts = Vec::new();
        if sub_count > 0 {
            parts.push(loc::tf("Str.FolderDelete.SubFolders", &[&sub_count]));
        }
        if shortcut_count > 0 {
            parts.push(loc::tf("Str.FolderDelete.Shortcuts", &[&shortcut_count]));
        }
        let detail = if parts.is_empty() {
            loc::t("Str.FolderDelete.Empty")
        } else {
            loc::tf("Str.FolderDelete.Contains", &[&parts.join(&loc::t("Str.Common.ListSeparator"))])
        };
        Self { folder, name, detail, shortcut_count, delete_shortcuts: false, done: false }
    }

    pub fn into_outcome(self) -> Outcome {
        if self.done { Outcome::DeleteFolder { folder: self.folder, delete_shortcuts: self.delete_shortcuts } } else { Outcome::None }
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) -> bool {
        let p = core.palette;
        let mut close = false;
        egui::Frame::new().inner_margin(egui::Margin::same(20)).show(ui, |ui| {
            ui.set_width(ui.ctx().content_rect().width() - 40.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(
                egui::Label::new(widgets::rich(&loc::tf("Str.Common.ConfirmDeleteOne", &[&self.name]), style::bold(14.0), p.text_primary))
                    .wrap(),
            );
            ui.add_space(8.0);
            widgets::caption(ui, &p, &self.detail);

            // 中身が無ければ選択肢を出す意味がない
            if self.shortcut_count > 0 {
                ui.add_space(16.0);
                card_like(ui, &p, 14, 8, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    header(ui, &p, &loc::t("Str.FolderDelete.ContentsLabel"));
                    ui.add_space(2.0);
                    if ui.radio(!self.delete_shortcuts, loc::t("Str.FolderDelete.Keep")).clicked() {
                        self.delete_shortcuts = false;
                    }
                    ui.add_space(8.0);
                    if ui.radio(self.delete_shortcuts, loc::t("Str.FolderDelete.DeleteToo")).clicked() {
                        self.delete_shortcuts = true;
                    }
                });
            }

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button_ex(ui, &p, ButtonKind::Danger, &loc::t("Str.FolderDelete.Confirm"), true, Some(100.0)).clicked() {
                        self.done = true;
                        close = true;
                    }
                    if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Cancel"), true, Some(100.0)).clicked() {
                        close = true;
                    }
                });
            });
        });
        close || take_escape(ui)
    }
}

// =================================================================== 1 行入力

/// フォルダ名などを 1 行入力してもらう共通ダイアログ。
pub struct TextInput {
    purpose: TextPurpose,
    title: String,
    label: String,
    value: String,
    error: Option<String>,
    done: bool,
    first: bool,
}

impl TextInput {
    pub fn new(purpose: TextPurpose, title: String, label: String, initial: String) -> Self {
        Self { purpose, title, label, value: initial, error: None, done: false, first: true }
    }

    pub fn title(&self) -> String {
        self.title.clone()
    }

    pub fn into_outcome(self) -> Outcome {
        if self.done { Outcome::TextEntered { purpose: self.purpose, value: self.value.trim().to_string() } } else { Outcome::None }
    }

    fn commit(&mut self) -> bool {
        if self.value.trim().is_empty() {
            self.error = Some(loc::t("Str.TextInput.Required"));
            self.first = true; // 入力欄へフォーカスを戻す
            return false;
        }
        self.done = true;
        true
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core) -> bool {
        let p = core.palette;
        let mut close = false;
        egui::Frame::new().inner_margin(egui::Margin::same(20)).show(ui, |ui| {
            let width = ui.ctx().content_rect().width() - 40.0;
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            header(ui, &p, &self.label);
            let id = egui::Id::unique("text-input");
            let response = widgets::framed_text_edit(
                ui,
                &p,
                vec2(width, 34.0),
                egui::TextEdit::singleline(&mut self.value)
                    .id(id)
                    .font(style::regular(14.0))
                    .margin(vec2(8.0, 6.0))
                    .align(egui::Align2::LEFT_CENTER),
            );
            if self.first {
                self.first = false;
                response.request_focus();
                select_all(ui, id, &self.value);
            }
            if let Some(error) = &self.error {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    error_text(ui, &p, error, style::CAPTION);
                });
            }
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button_ex(ui, &p, ButtonKind::Primary, "OK", true, Some(92.0)).clicked() && self.commit() {
                        close = true;
                    }
                    if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Cancel"), true, Some(92.0)).clicked() {
                        close = true;
                    }
                });
            });
        });
        if close {
            return true;
        }
        // IME 変換確定の Enter は egui に届かないので、ここで閉じてしまうことはない
        if take_enter(ui) && self.commit() {
            return true;
        }
        take_escape(ui)
    }
}

// ============================================================== インポート方法

/// インポート方法（追加 / 置き換え）を選ぶダイアログ（§27）。
pub struct ImportModeDialog {
    path: PathBuf,
    manifest: ExportManifest,
    mode: ImportMode,
    done: bool,
}

impl ImportModeDialog {
    pub fn new(path: PathBuf, manifest: ExportManifest) -> Self {
        Self { path, manifest, mode: ImportMode::Merge, done: false }
    }

    pub fn into_outcome(self) -> Outcome {
        if self.done { Outcome::Import { path: self.path, mode: self.mode } } else { Outcome::None }
    }

    fn exported_at_text(&self) -> String {
        let exported = match chrono::DateTime::parse_from_rfc3339(&self.manifest.exported_at) {
            Ok(dt) => dt.with_timezone(&chrono::Local).format("%Y/%m/%d %H:%M").to_string(),
            Err(_) => self.manifest.exported_at.clone(),
        };
        loc::tf("Str.ImportMode.ExportedAt", &[&exported, &self.manifest.app_version])
    }

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core, window: &Arc<Window>) -> bool {
        let p = core.palette;
        let mut close = false;
        egui::Frame::new().inner_margin(egui::Margin::same(22)).show(ui, |ui| {
            let width = ui.ctx().content_rect().width() - 44.0;
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(widgets::rich(&loc::t("Str.ImportMode.Heading"), style::bold(15.0), p.text_primary));
            ui.add_space(12.0);
            card_like(ui, &p, 14, 8, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let m = &self.manifest;
                ui.add(egui::Label::new(loc::tf("Str.ImportMode.Summary", &[&m.folder_count, &m.shortcut_count, &m.tag_count])).wrap());
                ui.add_space(6.0);
                widgets::caption(ui, &p, &self.exported_at_text());
            });

            ui.add_space(18.0);
            header(ui, &p, &loc::t("Str.ImportMode.Method"));
            mode_option(ui, &p, &mut self.mode, ImportMode::Merge, "Str.ImportMode.Merge", "Str.ImportMode.MergeNote");
            ui.add_space(12.0);
            mode_option(ui, &p, &mut self.mode, ImportMode::Replace, "Str.ImportMode.Replace", "Str.ImportMode.ReplaceNote");

            ui.add_space(16.0);
            egui::Frame::new().fill(p.accent_soft).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(12, 10)).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(widgets::rich(&loc::t("Str.ImportMode.BackupNote"), style::regular(11.5), p.text_secondary))
                            .wrap(),
                    );
                },
            );

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button_ex(ui, &p, ButtonKind::Primary, &loc::t("Str.ImportMode.Title"), true, Some(100.0)).clicked() {
                        let ok = self.mode != ImportMode::Replace
                            || confirm(
                                window,
                                &loc::t("Str.ImportMode.ReplaceConfirm"),
                                &loc::t("Str.ImportMode.ReplaceConfirmTitle"),
                                MsgIcon::Warning,
                            );
                        if ok {
                            self.done = true;
                            close = true;
                        }
                    }
                    if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Cancel"), true, Some(100.0)).clicked() {
                        close = true;
                    }
                });
            });
        });
        close || take_escape(ui)
    }
}

/// ラジオボタン + 太字の見出し + 説明。
fn mode_option(ui: &mut Ui, p: &Palette, mode: &mut ImportMode, value: ImportMode, title: &str, note: &str) {
    let width = ui.available_width();
    let top = ui.cursor().top();
    let response = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (rect, _) = ui.allocate_exact_size(vec2(24.0, 20.0), Sense::hover());
        let center = pos2(rect.left() + 8.0, rect.center().y);
        let selected = *mode == value;
        ui.painter().circle(center, 7.0, p.input_background, Stroke::new(1.0, if selected { p.accent } else { p.border_strong }));
        if selected {
            ui.painter().circle_filled(center, 4.0, p.accent);
        }
        ui.vertical(|ui| {
            ui.set_width(width - 24.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(widgets::rich(&loc::t(title), style::bold(style::BODY), p.text_primary));
            ui.add_space(2.0);
            widgets::caption(ui, p, &loc::t(note));
        });
    });
    let rect = Rect::from_min_max(pos2(response.response.rect.left(), top), response.response.rect.max);
    if ui.interact(rect, ui.scope_id().with(("mode", title)), Sense::click()).clicked() {
        *mode = value;
    }
}

//! Edge / Chrome などのお気に入りを取り込むダイアログ（C# 版 BookmarkImportWindow と同じ構成）。
//!
//! ブラウザのファイルは読み取るだけで、一切書き込まない。
//! 取り込み先・フォルダ構成の扱い・重複の扱いを選んでから実行する。

use std::collections::HashSet;
use std::sync::Arc;

use egui::{Align, CornerRadius, Layout, Stroke, Ui, vec2};
use winit::window::Window;

use super::{Outcome, folder_choices, folder_combo, header, message, take_escape};
use crate::app::{Command, Core};
use crate::bookmarks::{self, BookmarkNode, BookmarkProfile};
use crate::loc;
use crate::model::Shortcut;
use crate::platform::MsgIcon;
use crate::target;
use crate::ui::style::{self, Palette};
use crate::ui::widgets::{self, ButtonKind};
use crate::util::fold;

pub struct BookmarkDialog {
    profiles: Vec<BookmarkProfile>,
    profile: usize,
    roots: Vec<BookmarkNode>,
    checks: Vec<bool>,
    new_folder: bool,
    new_folder_name: String,
    folder_choices: Vec<(Option<String>, String)>,
    folder: Option<String>,
    keep_structure: bool,
    skip_duplicates: bool,
    add_tag: bool,
    tag_name: String,
    status: String,
    imported: usize,
    skipped: usize,
    done: bool,
}

impl BookmarkDialog {
    pub fn new(core: &Core, current_folder: Option<String>) -> Self {
        let choices = folder_choices(core);
        let folder = if choices.iter().any(|(id, _)| *id == current_folder) { current_folder } else { None };
        let profiles = bookmarks::find_profiles();
        let mut dialog = Self {
            profiles,
            profile: 0,
            roots: Vec::new(),
            checks: Vec::new(),
            new_folder: true,
            new_folder_name: String::new(),
            folder_choices: choices,
            folder,
            keep_structure: true,
            skip_duplicates: true,
            add_tag: false,
            tag_name: String::new(),
            status: String::new(),
            imported: 0,
            skipped: 0,
            done: false,
        };
        if dialog.profiles.is_empty() {
            dialog.status = loc::t("Str.Bookmark.NoBrowsers");
        } else {
            dialog.load_profile(0);
        }
        dialog
    }

    pub fn into_outcome(self) -> Outcome {
        if self.done { Outcome::BookmarksImported { imported: self.imported, skipped: self.skipped } } else { Outcome::None }
    }

    fn load_profile(&mut self, index: usize) {
        let Some(profile) = self.profiles.get(index).cloned() else { return };
        self.profile = index;
        self.status.clear();
        self.roots = match bookmarks::read_roots(&profile.file_path) {
            Ok(roots) => roots,
            Err(e) => {
                crate::log::error(&format!("お気に入りを読めませんでした: {} ({e})", profile.file_path.display()));
                self.status = loc::tf("Str.Bookmark.ReadFailed", &[&e]);
                Vec::new()
            }
        };
        // 既定のフォルダ名を、選んだブラウザに合わせる
        self.new_folder_name = loc::tf("Str.Bookmark.DefaultFolderName", &[&profile.browser_name]);
        self.tag_name = profile.browser_name.clone();
        self.checks = self.roots.iter().map(|r| r.link_count() > 0).collect();
    }

    fn selected_roots(&self) -> Vec<&BookmarkNode> {
        self.roots.iter().zip(&self.checks).filter(|(_, c)| **c).map(|(r, _)| r).collect()
    }

    fn summary(&self) -> (usize, String) {
        let selected = self.selected_roots();
        let links: usize = selected.iter().map(|r| r.link_count()).sum();
        let folders: usize = if self.keep_structure { selected.iter().map(|r| 1 + r.folder_count()).sum() } else { 0 };
        let text = if folders > 0 {
            loc::tf("Str.Bookmark.Summary", &[&links, &folders])
        } else {
            loc::tf("Str.Bookmark.SummaryNoFolders", &[&links])
        };
        (links, text)
    }

    // --------------------------------------------------------------- 取り込み

    /// 取り込んだら true（閉じる）。
    fn import(&mut self, core: &mut Core, window: &Arc<Window>) -> bool {
        let selected: Vec<BookmarkNode> = self.selected_roots().into_iter().cloned().collect();
        if selected.is_empty() {
            return false;
        }

        // 取り込み先フォルダを決める。新規作成を選んでいればここで作る
        let destination = if self.new_folder {
            let name = self.new_folder_name.trim();
            if name.is_empty() {
                self.status = loc::t("Str.Bookmark.FolderNameRequired");
                return false;
            }
            match core.store.create_folder(name, None) {
                Ok(id) => Some(id),
                Err(e) => {
                    self.status = e.to_string();
                    return false;
                }
            }
        } else {
            self.folder.clone()
        };

        let keep = self.keep_structure;
        let skip = self.skip_duplicates;
        let tag = if self.add_tag { self.tag_name.trim().to_string() } else { String::new() };
        // 既存の URL は大文字小文字を無視して比較する
        let mut existing: HashSet<String> =
            if skip { core.store.shortcuts().iter().map(|s| fold(&s.target)).collect() } else { HashSet::new() };

        // 画面の更新は最後の 1 回にまとめる（1 件ごとに一覧を作り直すと、件数に比例して遅くなる）
        core.store.begin_batch();
        let mut result = Ok(());
        for root in &selected {
            // 「ブックマーク バー」などの最上位フォルダも、構成を再現するなら作る
            let root_folder = if keep {
                match core.store.create_folder(&safe_folder_name(&root.name), destination.as_deref()) {
                    Ok(id) => Some(id),
                    Err(e) => {
                        result = Err(e.to_string());
                        break;
                    }
                }
            } else {
                destination.clone()
            };
            if let Err(e) = self.import_children(core, root, root_folder, keep, skip, &mut existing, &tag) {
                result = Err(e);
                break;
            }
        }
        core.store.end_batch();

        if let Err(e) = result {
            crate::log::error(&format!("お気に入りの取り込みに失敗しました。{e}"));
            message(window, &loc::tf("Str.Bookmark.ImportFailed", &[&e]), &loc::t("Str.Common.Error"), MsgIcon::Error);
        }
        crate::log::info(&format!("お気に入りを取り込みました: {} 件（スキップ {} 件）", self.imported, self.skipped));
        self.done = true;
        core.push(Command::RefreshTray);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn import_children(
        &mut self,
        core: &mut Core,
        folder: &BookmarkNode,
        folder_id: Option<String>,
        keep: bool,
        skip: bool,
        existing: &mut HashSet<String>,
        tag: &str,
    ) -> Result<(), String> {
        for child in &folder.children {
            if child.is_folder() {
                let child_folder = if keep {
                    Some(core.store.create_folder(&safe_folder_name(&child.name), folder_id.as_deref()).map_err(|e| e.to_string())?)
                } else {
                    folder_id.clone()
                };
                self.import_children(core, child, child_folder, keep, skip, existing, tag)?;
                continue;
            }
            let Some(url) = child.url.as_deref() else { continue };
            if skip && !existing.insert(fold(url)) {
                self.skipped += 1;
                continue;
            }
            let kind = target::detect(url);
            let item = Shortcut {
                title: child.name.clone(),
                target: target::normalize(url, kind),
                target_type: kind,
                folder_id: folder_id.clone(),
                tags: if tag.is_empty() { Vec::new() } else { vec![tag.to_string()] },
                ..Shortcut::new()
            };
            let probe = item.clone();
            core.store.add_shortcut(item).map_err(|e| e.to_string())?;
            // ブラウザから取り込んだ項目は、そのブラウザがアイコンを持っている可能性が高い
            let use_cache = core.settings.use_browser_icon_cache;
            let dark = core.is_dark();
            core.icons.request_favicon(&probe, false, use_cache, dark);
            self.imported += 1;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- 画面

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core, window: &Arc<Window>) -> bool {
        let p = core.palette;
        let (links, summary) = self.summary();
        let mut close = false;

        // ---- ボタン
        egui::Panel::bottom("bookmark-buttons")
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 16, bottom: 16 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let enabled = links > 0 && !self.profiles.is_empty();
                        if widgets::button_ex(ui, &p, ButtonKind::Primary, &loc::t("Str.Bookmark.Import"), enabled, Some(100.0)).clicked()
                            && self.import(core, window)
                        {
                            close = true;
                        }
                        if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Cancel"), true, Some(100.0)).clicked() {
                            close = true;
                        }
                    });
                });
            });

        // ---- 取り込み先・オプション
        egui::Panel::bottom("bookmark-options")
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 16, bottom: 0 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                header(ui, &p, &loc::t("Str.Bookmark.Destination"));
                let label_w = 150.0f32;
                ui.horizontal(|ui| {
                    ui.allocate_ui(vec2(label_w, 30.0), |ui| {
                        if ui.radio(self.new_folder, loc::t("Str.Bookmark.NewFolder")).clicked() {
                            self.new_folder = true;
                        }
                    });
                    ui.add_enabled_ui(self.new_folder, |ui| {
                        let w = ui.available_width();
                        widgets::text_field(ui, &mut self.new_folder_name, egui::Id::unique("bookmark-folder-name"), w);
                    });
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.allocate_ui(vec2(label_w, 30.0), |ui| {
                        if ui.radio(!self.new_folder, loc::t("Str.Bookmark.ExistingFolder")).clicked() {
                            self.new_folder = false;
                        }
                    });
                    let w = ui.available_width();
                    folder_combo(ui, "bookmark-folder", &self.folder_choices, &mut self.folder, w - 8.0, !self.new_folder);
                });

                ui.add_space(14.0);
                ui.checkbox(&mut self.keep_structure, loc::t("Str.Bookmark.KeepStructure"));
                ui.add_space(10.0);
                ui.checkbox(&mut self.skip_duplicates, loc::t("Str.Bookmark.SkipDuplicates"));
                ui.add_space(10.0);
                ui.checkbox(&mut self.add_tag, loc::t("Str.Bookmark.AddTag"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(25.0);
                    ui.add_enabled_ui(self.add_tag, |ui| {
                        widgets::text_field(ui, &mut self.tag_name, egui::Id::unique("bookmark-tag"), 200.0);
                    });
                });
                if !self.status.is_empty() {
                    ui.add_space(12.0);
                    widgets::caption(ui, &p, &self.status);
                }
            });

        // ---- 取り込み元・範囲
        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 18, bottom: 0 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                header(ui, &p, &loc::t("Str.Bookmark.Source"));
                let names: Vec<String> = self.profiles.iter().map(BookmarkProfile::display_name).collect();
                let mut index = self.profile;
                let current = names.get(index).cloned().unwrap_or_default();
                let w = ui.available_width();
                ui.add_enabled_ui(!self.profiles.is_empty(), |ui| {
                    egui::ComboBox::from_id_salt("bookmark-profile").selected_text(current).width(w - 8.0).show_ui(ui, |ui| {
                        for (i, name) in names.iter().enumerate() {
                            ui.selectable_value(&mut index, i, name);
                        }
                    });
                });
                if index != self.profile {
                    self.load_profile(index);
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    widgets::section_header(ui, &p, &loc::t("Str.Bookmark.Range"));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(widgets::rich(&summary, style::regular(style::CAPTION), p.text_secondary));
                    });
                });
                ui.add_space(6.0);
                self.root_list(ui, &p);
            });
        });

        if close {
            return true;
        }
        take_escape(ui)
    }

    fn root_list(&mut self, ui: &mut Ui, p: &Palette) {
        let height = ui.available_height().max(60.0);
        egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(height - 18.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if self.roots.is_empty() {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(6.0);
                            widgets::caption(ui, p, &loc::t("Str.Bookmark.NoBookmarks"));
                        });
                        return;
                    }
                    for (index, root) in self.roots.iter().enumerate() {
                        let count = root.link_count();
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.add_enabled_ui(count > 0, |ui| {
                                let mut job = egui::text::LayoutJob::default();
                                job.append("📁  ", 0.0, egui::TextFormat::simple(style::regular(style::BODY), p.text_primary));
                                let name = if root.name.is_empty() { loc::t("Str.Bookmark.Untitled") } else { root.name.clone() };
                                job.append(&name, 0.0, egui::TextFormat::simple(style::bold(style::BODY), p.text_primary));
                                job.append(
                                    &loc::tf("Str.Bookmark.RootCount", &[&count]),
                                    0.0,
                                    egui::TextFormat::simple(style::regular(style::BODY), p.text_secondary),
                                );
                                ui.checkbox(&mut self.checks[index], job);
                            });
                        });
                        ui.add_space(5.0);
                    }
                });
            });
    }
}

/// ブラウザ側で名前のないフォルダがあっても破綻しないようにする。
fn safe_folder_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() { loc::t("Str.Bookmark.Untitled") } else { trimmed.to_string() }
}

//! ランチャー（C# 版 LauncherWindow と同じ構成）。
//!
//! 「Ctrl + Space → 検索 → Enter」の 3 ステップで目的のものを開くための画面。
//! ウィンドウは閉じずに隠しておき、次回は表示するだけにしている（§59）。

use egui::{Align, Color32, CornerRadius, Key, Layout, Modifiers, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::app::{Command, Core, Owner};
use crate::loc;
use crate::ui::style::{self, Palette};
use crate::ui::widgets;

/// ウィンドウの幅（C# 版は影の余白 18 を含めて 660。こちらは影を Windows が描くので中身だけ）。
pub const WIDTH: f32 = 624.0;
/// 高さの上限（結果の一覧は最大 430 で、それ以上はスクロール）
pub const MAX_HEIGHT: f32 = 640.0;
const LIST_MAX_HEIGHT: f32 = 430.0;
const ROW_HEIGHT: f32 = 50.0;

#[derive(Default)]
pub struct LauncherState {
    query: String,
    results: Vec<String>,
    header: String,
    selected: usize,
    /// 結果を作ったときの (データの版, 検索語)
    key: Option<(u64, String)>,
    focus_query: bool,
    scroll_to_selected: bool,
}

impl LauncherState {
    /// 表示する直前に呼ぶ（検索欄を空にして結果を作り直す）。
    pub fn reset(&mut self, core: &mut Core) {
        self.query.clear();
        self.key = None;
        self.selected = 0;
        self.focus_query = true;
        self.scroll_to_selected = true;
        self.update_results(core);
    }

    /// 隠したときに呼ぶ。
    pub fn clear(&mut self) {
        self.results.clear();
        self.key = None;
    }

    fn update_results(&mut self, core: &mut Core) {
        let key = (core.store.data_version, self.query.clone());
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let max = core.settings.launcher_max_results.max(1) as usize;
        let revision = core.store.revision;
        let (items, header) = match crate::search::search(core.store.shortcuts(), &self.query, &mut core.search, revision, max) {
            Some(hits) => {
                let items: Vec<String> = hits.into_iter().map(|h| h.id).collect();
                let header = loc::tf("Str.Launcher.Results", &[&items.len()]);
                (items, header)
            }
            None if core.settings.show_recent_in_launcher => {
                let items = crate::search::recent(core.store.shortcuts(), max);
                let header = if items.is_empty() { String::new() } else { loc::t("Str.Launcher.Recent") };
                (items, header)
            }
            None => (Vec::new(), String::new()),
        };
        // 入力が変わったら先頭を選ぶ（C# 版と同じ）
        let query_changed = self.key.as_ref().is_none_or(|(_, q)| *q != self.query);
        if query_changed || self.selected >= items.len() {
            self.selected = 0;
            self.scroll_to_selected = true;
        }
        self.results = items;
        self.header = header;
        self.key = Some(key);
    }

    fn move_selection(&mut self, delta: i32) {
        if self.results.is_empty() {
            return;
        }
        let index = (self.selected as i32 + delta).clamp(0, self.results.len() as i32 - 1);
        self.selected = index as usize;
        self.scroll_to_selected = true;
    }
}

pub fn ui(ui: &mut Ui, st: &mut LauncherState, core: &mut Core) {
    let p = core.palette;
    // 背景（角丸と影は Windows が付ける）
    let screen = ui.ctx().content_rect();
    ui.painter().rect_filled(screen, CornerRadius::ZERO, p.header_background);

    st.update_results(core);
    let typing_keys = handle_keys(ui, st, core);
    if typing_keys {
        return;
    }

    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ui.set_width(screen.width());

    // ---- 検索欄
    egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 18, top: 16, bottom: 12 }).show(ui, |ui| {
        let id = egui::Id::unique("launcher-query");
        let before = st.query.clone();
        let width = ui.available_width();
        let (response, _) = widgets::search_box(ui, &p, &mut st.query, &loc::t("Str.Launcher.Placeholder"), id, 19.0, width, false);
        if st.focus_query {
            st.focus_query = false;
            response.request_focus();
        }
        if st.query != before {
            st.update_results(core);
        }
    });
    separator(ui, &p);

    // ---- 結果の見出し
    if !st.header.is_empty() {
        egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 10, bottom: 0 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::section_header(ui, &p, &st.header);
        });
    }

    // ---- 結果
    if st.results.is_empty() {
        let text = if !st.query.trim().is_empty() {
            loc::t("Str.Launcher.NoMatch")
        } else if core.store.shortcut_count() == 0 {
            loc::t("Str.Launcher.NoShortcuts")
        } else {
            loc::t("Str.Launcher.TypeKeyword")
        };
        egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 4, bottom: 18 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::caption(ui, &p, &text);
        });
    } else {
        egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 8 }).show(ui, |ui| {
            result_list(ui, st, core, &p);
        });
    }

    // ---- 下の案内
    let footer = egui::Frame::new().fill(p.sidebar_background).inner_margin(egui::Margin::symmetric(18, 7)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(widgets::rich(&loc::t("Str.Launcher.Footer"), style::regular(style::CAPTION), p.text_secondary));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let total = loc::tf("Str.Launcher.Total", &[&core.store.shortcut_count()]);
                ui.label(widgets::rich(&total, style::regular(style::CAPTION), p.text_secondary));
            });
        });
    });
    let _ = footer;

    // 枠線（C# 版の BorderStrongBrush の 1px 枠）
    ui.painter().rect_stroke(screen.shrink(0.5), CornerRadius::ZERO, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
}

fn separator(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.border);
}

fn result_list(ui: &mut Ui, st: &mut LauncherState, core: &mut Core, p: &Palette) {
    let total_height = st.results.len() as f32 * ROW_HEIGHT;
    let mut area = egui::ScrollArea::vertical().id_salt("launcher-results").max_height(LIST_MAX_HEIGHT).auto_shrink([false, true]);
    if std::mem::take(&mut st.scroll_to_selected) {
        let id = ui.make_persistent_id("launcher-results");
        let current = egui::scroll_area::State::load(ui.ctx(), id).map(|s| s.clamped_offset().y).unwrap_or(0.0);
        let view = LIST_MAX_HEIGHT.min(total_height);
        let top = st.selected as f32 * ROW_HEIGHT;
        if top < current {
            area = area.vertical_scroll_offset(top);
        } else if top + ROW_HEIGHT > current + view {
            area = area.vertical_scroll_offset(top + ROW_HEIGHT - view);
        }
    }
    let mut launch: Option<usize> = None;
    let mut menu_index: Option<usize> = None;
    area.show_rows(ui, ROW_HEIGHT, st.results.len(), |ui, range| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for index in range {
            let Some(item) = st.results.get(index).and_then(|id| core.store.find_shortcut(id)).cloned() else {
                ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
                continue;
            };
            let (outer, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
            let row = Rect::from_min_max(pos2(outer.left() + 4.0, outer.top() + 1.0), pos2(outer.right() - 4.0, outer.bottom() - 1.0));
            let selected = index == st.selected;
            let fill = if selected {
                p.accent_soft
            } else if response.hovered() {
                p.surface_hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(row, CornerRadius::same(8), fill);

            let inner = row.shrink2(vec2(10.0, 8.0));
            let icon_rect = Rect::from_center_size(pos2(inner.left() + 2.0 + 13.0, inner.center().y), vec2(26.0, 26.0));
            let icon = core.icons.resolve(&item, core.settings.use_browser_icon_cache, core.is_dark());
            widgets::paint_icon(ui, icon_rect, &icon);

            let right_w = 220.0f32.min(inner.width() * 0.35);
            let text_left = icon_rect.right() + 14.0;
            let text_right = inner.right() - right_w - 18.0;
            paint_line(
                ui,
                pos2(text_left, inner.center().y - 9.0),
                item.display_title(),
                style::bold(14.0),
                p.text_primary,
                text_right - text_left,
                false,
            );
            paint_line(
                ui,
                pos2(text_left, inner.center().y + 10.0),
                &item.target,
                style::regular(style::CAPTION),
                p.text_secondary,
                text_right - text_left,
                false,
            );
            paint_line(
                ui,
                pos2(inner.right() - 4.0, inner.center().y - 9.0),
                &item.tags_text(),
                style::regular(style::CAPTION),
                p.text_secondary,
                right_w,
                true,
            );
            paint_line(
                ui,
                pos2(inner.right() - 4.0, inner.center().y + 10.0),
                &item.folder_path_text,
                style::regular(style::CAPTION),
                p.text_secondary,
                right_w,
                true,
            );

            if response.clicked() {
                st.selected = index;
                launch = Some(index);
            }
            if response.secondary_clicked() {
                st.selected = index;
            }
            let id = item.id.clone();
            response.context_menu(|ui| {
                ui.set_min_width(200.0);
                menu_index = Some(index);
                launcher_menu(ui, core, &id);
            });
        }
    });
    let _ = menu_index;
    if let Some(index) = launch {
        launch_index(st, core, index);
    }
}

fn launcher_menu(ui: &mut Ui, core: &mut Core, id: &str) {
    if ui.button(loc::t("Str.Menu.Open")).clicked() {
        core.push(Command::HideLauncher);
        core.push(Command::Launch { ids: vec![id.to_string()], from_main: false });
        ui.close();
    }
    if ui.button(loc::t("Str.Menu.Edit")).clicked() {
        core.push(Command::HideLauncher);
        core.push(Command::OpenEdit { existing: Some(id.to_string()), folder: None, initial: None, owner: Owner::Main });
        ui.close();
    }
    if ui.button(loc::t("Str.Menu.CopyTarget")).clicked() {
        if let Some(item) = core.store.find_shortcut(id)
            && !crate::platform::write_clipboard_text(&item.target)
        {
            crate::log::warn("クリップボードへのコピーに失敗しました。");
        }
        ui.close();
    }
    if ui.button(loc::t("Str.Menu.Reveal")).clicked() {
        core.push(Command::HideLauncher);
        core.push(Command::Reveal { id: id.to_string() });
        ui.close();
    }
    ui.separator();
    if ui.button(loc::t("Str.Common.Delete")).clicked() {
        // 確認のメッセージを出すとランチャーは隠れるので、先に隠しておく
        core.push(Command::HideLauncher);
        core.push(Command::DeleteShortcuts { ids: vec![id.to_string()] });
        ui.close();
    }
}

fn launch_index(st: &mut LauncherState, core: &mut Core, index: usize) {
    let Some(id) = st.results.get(index).cloned() else { return };
    // 先に閉じてから起動すると、開いたアプリが最前面に出て自然に見える
    core.push(Command::HideLauncher);
    core.push(Command::Launch { ids: vec![id], from_main: false });
}

/// 1 行の文字（はみ出したら「…」）。right_align なら pos を右端とみなす。
fn paint_line(ui: &Ui, pos: egui::Pos2, text: &str, font: egui::FontId, color: Color32, max_width: f32, right_align: bool) {
    if text.is_empty() || max_width <= 4.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
    let galley = ui.painter().layout_job(job);
    let x = if right_align { pos.x - galley.size().x } else { pos.x };
    ui.painter().galley(pos2(x, pos.y - galley.size().y / 2.0), galley, color);
}

/// キー操作（検索欄に文字が入る前に処理する）。ランチャーを隠したら true。
fn handle_keys(ui: &mut Ui, st: &mut LauncherState, core: &mut Core) -> bool {
    // IME が処理中のキー（変換のスペースや確定の Enter）は egui に届かないので、ここでは気にしなくてよい
    let (escape, down, up, page_down, page_up, enter, tab, shift_tab, ctrl_n) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::Escape),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::PageDown),
            i.consume_key(Modifiers::NONE, Key::PageUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Tab),
            i.consume_key(Modifiers::SHIFT, Key::Tab),
            i.consume_key(Modifiers::COMMAND, Key::N),
        )
    });
    if escape {
        core.push(Command::HideLauncher);
        return true;
    }
    if ctrl_n {
        // コピーした URL / パスを入れた状態で登録画面を開く
        core.push(Command::HideLauncher);
        core.push(Command::RegisterFromClipboard);
        return true;
    }
    if down || tab {
        st.move_selection(1);
    }
    if up || shift_tab {
        st.move_selection(-1);
    }
    if page_down {
        st.move_selection(5);
    }
    if page_up {
        st.move_selection(-5);
    }
    if enter && !st.results.is_empty() {
        let index = st.selected;
        launch_index(st, core, index);
        return true;
    }
    false
}

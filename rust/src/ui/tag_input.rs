//! YouTube 風のタグ入力（C# 版 TagInputControl と同じ動き、§5）。
//!
//! 文字を入力して Space を押すとタグチップとして確定する。
//!
//! 日本語 IME への配慮（§32）:
//!   - IME が変換中のキー（変換の Space・確定の Enter）は egui に届かない（egui-winit が除外する）ので、
//!     「かいはつ」→ Space で変換、の操作がタグの確定にならない。
//!   - IME が全角スペースを直接入力した場合に備え、入力後の文字列でも
//!     半角 / 全角スペースと読点・カンマを区切りとして扱う。

use egui::{Color32, CornerRadius, Event, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::loc;
use crate::ui::style::{self, Palette};
use crate::util::{contains_ignore_case, eq_ignore_case, starts_with_ignore_case};

/// タグの区切りとして扱う文字。全角スペース・読点も含める。
const SEPARATORS: [char; 4] = [' ', '\u{3000}', ',', '、'];
const MAX_TAG_CHARS: usize = 60;

pub struct TagInput {
    id: egui::Id,
    tags: Vec<String>,
    available: Vec<String>,
    text: String,
    suggestions: Vec<String>,
    selected_suggestion: Option<usize>,
    /// 候補の一覧を出していた場所（前のフレーム）。候補のクリックで入力途中の文字を確定させないため
    popup_rect: Option<Rect>,
    /// 枠の場所（前のフレーム）。枠のどこをクリックしても入力欄にフォーカスするため
    box_rect: Option<Rect>,
    focus_requested: bool,
}

/// show の結果。
pub struct TagInputResponse {
    /// Enter / Escape をタグ入力が使った（ダイアログの既定ボタン・キャンセルに回さない）
    pub consumed_enter: bool,
    pub consumed_escape: bool,
}

impl TagInput {
    pub fn new(id_salt: &str) -> Self {
        Self {
            id: egui::Id::unique(("tag-input", id_salt)),
            tags: Vec::new(),
            available: Vec::new(),
            text: String::new(),
            suggestions: Vec::new(),
            selected_suggestion: None,
            popup_rect: None,
            box_rect: None,
            focus_requested: false,
        }
    }

    /// 入力補完に使う既存タグの一覧。
    pub fn set_available(&mut self, tags: Vec<String>) {
        let mut list: Vec<String> = Vec::new();
        for t in tags {
            if !list.iter().any(|x| eq_ignore_case(x, &t)) {
                list.push(t);
            }
        }
        self.available = list;
    }

    pub fn set_tags(&mut self, tags: &[String]) {
        self.tags.clear();
        for tag in tags {
            let name = tag.trim();
            if name.is_empty() || self.tags.iter().any(|t| eq_ignore_case(t, name)) {
                continue;
            }
            self.tags.push(name.to_string());
        }
    }

    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// 入力欄に残っている文字もタグとして確定する（保存直前に呼ぶ）。
    pub fn commit_pending(&mut self) {
        self.commit_current_text();
        self.hide_suggestions();
    }

    fn edit_id(&self) -> egui::Id {
        self.id.with("edit")
    }

    fn commit_current_text(&mut self) -> bool {
        let text = self.text.trim().to_string();
        if text.is_empty() {
            return false;
        }
        let mut added = false;
        for part in text.split(SEPARATORS).filter(|p| !p.trim().is_empty()) {
            added |= self.add_tag(part);
        }
        self.text.clear();
        self.hide_suggestions();
        added
    }

    fn add_tag(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let name: String = name.chars().take(MAX_TAG_CHARS).collect();
        // 大文字小文字違いの同じタグは登録しない（§5）
        if self.tags.iter().any(|t| eq_ignore_case(t, &name)) {
            return false;
        }
        self.tags.push(name);
        true
    }

    fn remove_tag(&mut self, name: &str) {
        self.tags.retain(|t| !eq_ignore_case(t, name));
    }

    fn hide_suggestions(&mut self) {
        self.suggestions.clear();
        self.selected_suggestion = None;
    }

    fn update_suggestions(&mut self) {
        let text = self.text.trim();
        if text.is_empty() || self.available.is_empty() {
            self.hide_suggestions();
            return;
        }
        let mut matches: Vec<&String> = self
            .available
            .iter()
            .filter(|t| !self.tags.iter().any(|x| eq_ignore_case(x, t)))
            .filter(|t| contains_ignore_case(t, text))
            .collect();
        // 前方一致を先に、次に短い順（C# 版と同じ。並べ替えは安定なので同順位は元の順）
        matches.sort_by_key(|t| (if starts_with_ignore_case(t, text) { 0 } else { 1 }, t.chars().count()));
        self.suggestions = matches.into_iter().take(8).cloned().collect();
        self.selected_suggestion = None;
    }

    /// 入力欄を描く。
    pub fn show(&mut self, ui: &mut Ui, p: &Palette) -> TagInputResponse {
        let mut result = TagInputResponse { consumed_enter: false, consumed_escape: false };
        let edit_id = self.edit_id();
        let focused = ui.memory(|m| m.has_focus(edit_id));

        // ---- 入力欄に文字が入る前のキー操作
        if focused {
            self.handle_keys_before_edit(ui, &mut result);
        }

        // 枠のどこをクリックしても入力欄にフォーカスする（C# 版の OnRootMouseDown）。
        // チップの ✕ や入力欄のクリックを優先させるため、中身より先に登録する
        if let Some(rect) = self.box_rect
            && ui.interact(rect, self.id.with("box"), Sense::click()).clicked()
        {
            self.focus_requested = true;
        }

        // ---- 枠とチップ
        let width = ui.available_width();
        let frame = egui::Frame::new()
            .fill(p.input_background)
            .stroke(Stroke::new(1.0, if focused { p.accent } else { p.border_strong }))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(5, 4));
        let mut remove: Option<String> = None;
        let mut edit_response: Option<egui::Response> = None;
        let frame_response = frame.show(ui, |ui| {
            ui.set_width(width - 10.0);
            ui.set_min_height(28.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for tag in self.tags.clone() {
                    if chip(ui, p, &tag) {
                        remove = Some(tag);
                    }
                }
                let hint = if self.tags.is_empty() { loc::t("Str.Tag.InputPlaceholder") } else { String::new() };
                // 入力欄はチップの後ろに置く（残りの幅が 120 未満なら次の行へ）
                // available_width() は折り返した後の幅（行の全体）なので、今の行の残りの幅を使う
                let available = ui.available_size_before_wrap().x;
                let edit_width = if available >= 132.0 { available - 12.0 } else { (width - 24.0).max(120.0) };
                let response = ui.add_sized(
                    vec2(edit_width, 26.0),
                    egui::TextEdit::singleline(&mut self.text)
                        .id(edit_id)
                        .frame(egui::Frame::NONE)
                        .font(style::regular(style::BODY))
                        .margin(vec2(4.0, 3.0))
                        .hint_text(egui::RichText::new(hint).color(p.text_disabled))
                        .align(egui::Align2::LEFT_CENTER),
                );
                edit_response = Some(response);
            });
        });
        let box_rect = frame_response.response.rect;
        self.box_rect = Some(box_rect);
        if let Some(tag) = remove {
            self.remove_tag(&tag);
            self.focus_requested = true;
        }

        let Some(edit) = edit_response else { return result };
        if self.focus_requested {
            self.focus_requested = false;
            edit.request_focus();
            // カーソルを末尾へ
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                let end = egui::text::CCursor::new(self.text.chars().count());
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), edit_id);
            }
        }

        // ---- 入力後: 区切り文字（IME が全角スペースを直接入れた場合など）
        if edit.changed() {
            if self.text.contains(SEPARATORS) {
                let parts: Vec<String> = self.text.split(SEPARATORS).map(str::to_string).collect();
                for part in &parts[..parts.len() - 1] {
                    self.add_tag(part);
                }
                self.text = parts[parts.len() - 1].clone();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                    let end = egui::text::CCursor::new(self.text.chars().count());
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                    state.store(ui.ctx(), edit_id);
                }
            }
            self.update_suggestions();
        }

        // フォーカスが外れたら入力途中の文字を捨てず、タグとして確定する（候補をクリックした場合を除く）
        if edit.lost_focus() {
            let over_popup = self.popup_rect.is_some_and(|r| ui.input(|i| i.pointer.interact_pos().is_some_and(|pos| r.contains(pos))));
            if !over_popup {
                self.commit_current_text();
                self.hide_suggestions();
            }
        }

        // ---- 候補
        self.popup_rect = None;
        if !self.suggestions.is_empty() && (edit.has_focus() || edit.lost_focus()) {
            self.show_suggestions(ui, p, box_rect);
        }
        result
    }

    fn handle_keys_before_edit(&mut self, ui: &mut Ui, result: &mut TagInputResponse) {
        let popup_open = !self.suggestions.is_empty();
        let text_empty = self.text.is_empty();

        // Space: 入力中の文字をタグとして確定する（空欄での Space は何もしない）
        let space = ui.input_mut(|i| {
            let pressed = i.consume_key(Modifiers::NONE, Key::Space) | i.consume_key(Modifiers::SHIFT, Key::Space);
            if pressed {
                i.events.retain(|e| !matches!(e, Event::Text(t) if t == " "));
            }
            pressed
        });
        if space {
            self.commit_current_text();
        }

        // Enter: 候補を選んでいればそれを追加。入力中の文字があればタグとして確定。
        // どちらでもなければダイアログの既定ボタン（保存）へ回す
        if ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.is_none()) {
            if popup_open && let Some(index) = self.selected_suggestion {
                let picked = self.suggestions[index].clone();
                self.add_tag(&picked);
                self.text.clear();
                self.hide_suggestions();
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                result.consumed_enter = true;
            } else if self.commit_current_text() {
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                result.consumed_enter = true;
            }
        }

        // Tab: 確定してから次の入力欄へ
        if ui.input(|i| i.key_pressed(Key::Tab)) {
            self.commit_current_text();
            self.hide_suggestions();
        }

        // Backspace: 空欄なら直前のタグを消す
        if text_empty && !self.tags.is_empty() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace)) {
            self.tags.pop();
        }

        if popup_open {
            let (down, up, escape) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            let count = self.suggestions.len();
            if down {
                self.selected_suggestion = Some(self.selected_suggestion.map_or(0, |s| (s + 1).min(count - 1)));
            }
            if up {
                self.selected_suggestion = Some(self.selected_suggestion.map_or(0, |s| s.saturating_sub(1)));
            }
            if escape {
                self.hide_suggestions();
                result.consumed_escape = true;
            }
        }
    }

    fn show_suggestions(&mut self, ui: &mut Ui, p: &Palette, anchor: Rect) {
        let mut picked: Option<String> = None;
        let area = egui::Area::new(self.id.with("suggestions"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(anchor.left(), anchor.bottom() + 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(3))
                    .shadow(egui::Shadow { offset: [0, 2], blur: 14, spread: 0, color: Color32::from_black_alpha(64) })
                    .show(ui, |ui| {
                        ui.set_width(anchor.width() - 6.0);
                        egui::ScrollArea::vertical().max_height(210.0).show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                            for (index, tag) in self.suggestions.iter().enumerate() {
                                let galley = ui.painter().layout_no_wrap(tag.clone(), style::regular(style::BODY), p.text_primary);
                                let (rect, response) =
                                    ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y + 12.0), Sense::click());
                                let selected = self.selected_suggestion == Some(index);
                                let fill = if selected {
                                    p.accent_soft
                                } else if response.hovered() {
                                    p.surface_hover
                                } else {
                                    Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
                                ui.painter().galley(
                                    pos2(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0),
                                    galley,
                                    p.text_primary,
                                );
                                if response.clicked() {
                                    picked = Some(tag.clone());
                                }
                            }
                        });
                    });
            });
        self.popup_rect = Some(area.response.rect);
        if let Some(tag) = picked {
            self.add_tag(&tag);
            self.text.clear();
            self.hide_suggestions();
            self.focus_requested = true;
            ui.ctx().request_repaint();
        }
    }
}

/// タグのチップ。✕ が押されたら true。
fn chip(ui: &mut Ui, p: &Palette, tag: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(tag.to_string(), style::regular(style::BODY), p.tag_foreground);
    let close_size = 18.0;
    let size = vec2(10.0 + galley.size().x + 6.0 + close_size + 4.0, (galley.size().y + 6.0).max(24.0));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(11), p.tag_background);
    ui.painter().galley(pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, p.tag_foreground);
    let close_rect = Rect::from_center_size(pos2(rect.right() - 4.0 - close_size / 2.0, rect.center().y), vec2(close_size, close_size));
    let response = ui.interact(close_rect, ui.scope_id().with(("chip-close", tag)), Sense::click());
    if response.hovered() {
        ui.painter().rect(close_rect, CornerRadius::same(5), p.surface_hover, Stroke::NONE, StrokeKind::Inside);
    }
    ui.painter().text(close_rect.center(), egui::Align2::CENTER_CENTER, "✕", style::regular(10.0), p.text_secondary);
    let response = response.on_hover_text(loc::tf("Str.Tag.RemoveTooltip", &[&tag])).on_hover_cursor(egui::CursorIcon::PointingHand);
    response.clicked()
}

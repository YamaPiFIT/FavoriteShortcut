//! 画面で共通に使う部品（C# 版のボタンの種類・文字の種類・既定アイコンに合わせたもの）。

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::CubicBezierShape;
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::icons::IconRef;
use crate::model::TargetType;
use crate::ui::style::{self, Palette};

// ------------------------------------------------------------------ ボタン

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// 主ボタン（青地に白い太字）
    Primary,
    /// 補助ボタン（白地に枠線）
    Secondary,
    /// 控えめなボタン（背景なし）
    Ghost,
    /// 危険な操作（赤地）
    Danger,
}

/// C# 版のボタンと同じ見た目のボタン。
pub fn button(ui: &mut Ui, p: &Palette, kind: ButtonKind, text: &str) -> Response {
    button_ex(ui, p, kind, text, true, None)
}

pub fn button_ex(ui: &mut Ui, p: &Palette, kind: ButtonKind, text: &str, enabled: bool, min_width: Option<f32>) -> Response {
    let (font, fg) = match kind {
        ButtonKind::Primary => (style::bold(style::BODY), p.accent_foreground),
        ButtonKind::Danger => (style::bold(style::BODY), Color32::WHITE),
        ButtonKind::Secondary => (style::regular(style::BODY), p.text_primary),
        ButtonKind::Ghost => (style::regular(style::BODY), p.text_secondary),
    };
    let fg = if enabled { fg } else { p.text_disabled };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let padding = if kind == ButtonKind::Ghost { vec2(8.0, 5.0) } else { vec2(14.0, 6.0) };
    let mut size = galley.size() + padding * 2.0;
    size.y = size.y.max(30.0);
    if let Some(w) = min_width {
        size.x = size.x.max(w);
    }
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let hovered = enabled && response.hovered();
        let pressed = enabled && response.is_pointer_button_down_on();
        let (fill, stroke) = match kind {
            ButtonKind::Primary => (
                if !enabled {
                    p.border
                } else if hovered {
                    p.accent_hover
                } else {
                    p.accent
                },
                Stroke::NONE,
            ),
            ButtonKind::Danger => (if enabled { p.danger } else { p.border }, Stroke::NONE),
            ButtonKind::Secondary => (
                if pressed {
                    p.surface_selected
                } else if hovered {
                    p.surface_hover
                } else {
                    p.surface
                },
                Stroke::new(1.0, if hovered { p.accent } else { p.border_strong }),
            ),
            ButtonKind::Ghost => (
                if pressed {
                    p.surface_selected
                } else if hovered {
                    p.surface_hover
                } else {
                    Color32::TRANSPARENT
                },
                Stroke::NONE,
            ),
        };
        ui.painter().rect(rect, CornerRadius::same(6), fill, stroke, StrokeKind::Inside);
        let text_pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(text_pos, galley, fg);
    }
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

/// 表示切替のトグル（カード / リスト）。
pub fn toggle(ui: &mut Ui, p: &Palette, text: &str, selected: bool) -> Response {
    let font = if selected { style::bold(style::BODY) } else { style::regular(style::BODY) };
    let fg = if selected { p.accent } else { p.text_secondary };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let size = galley.size() + vec2(20.0, 10.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if selected {
        p.accent_soft
    } else if response.hovered() {
        p.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------- 文字の種類

/// 補足の文字（CaptionText: 11.5、薄い色）。
pub fn caption(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    ui.add(egui::Label::new(RichText::new(text).font(style::regular(style::CAPTION)).color(p.text_secondary)).wrap())
}

/// 見出し（SectionHeaderText: 11、太字、薄い色）。
pub fn section_header(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    ui.label(RichText::new(text).font(style::bold(style::SECTION)).color(p.text_secondary))
}

/// 1 行に収まらなければ「…」で切り詰める文字。
pub fn truncated(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    ui.add(egui::Label::new(text).truncate())
}

pub fn rich(text: &str, font: FontId, color: Color32) -> RichText {
    RichText::new(text).font(font).color(color)
}

// ------------------------------------------------------------------ アイコン

#[derive(Clone, Default)]
struct TextureCache(HashMap<String, egui::TextureHandle>);

/// アイコンを rect に描く。画像はウィンドウごとのテクスチャのキャッシュに載せる。
pub fn paint_icon(ui: &Ui, rect: Rect, icon: &IconRef) {
    match icon {
        IconRef::Image { key, image } => {
            let texture = texture_for(ui.ctx(), key, image);
            // 縦横比を保ったまま枠に収める
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let draw = Rect::from_center_size(rect.center(), size * scale);
            ui.painter().image(texture.id(), draw, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        IconRef::Default(kind) => paint_default_icon(ui.painter(), rect, *kind),
    }
}

fn texture_for(ctx: &egui::Context, key: &str, image: &Arc<egui::ColorImage>) -> egui::TextureHandle {
    let id = egui::Id::unique("icon-textures");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureCache>(id)).and_then(|c| c.0.get(key).cloned()) {
        return t;
    }
    let texture = ctx.load_texture(key, (**image).clone(), egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<TextureCache>(id);
        // 古いものがたまり続けないよう、多くなったら一度捨てる
        if cache.0.len() > 4000 {
            cache.0.clear();
        }
        cache.0.insert(key.to_string(), texture.clone());
    });
    texture
}

/// favicon が取得できないときや、種別が判定できないときの既定アイコン（C# 版と同じ形と色）。
/// 24×24 の座標で描いて、表示の大きさに合わせて拡大する。
pub fn paint_default_icon(painter: &egui::Painter, rect: Rect, kind: TargetType) {
    let s = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - vec2(12.0 * s, 12.0 * s);
    let p = |x: f32, y: f32| -> Pos2 { origin + vec2(x * s, y * s) };
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(p(x0, y0), p(x1, y1));
    let white = Stroke::new(1.4 * s, Color32::WHITE);

    match kind {
        TargetType::Web => {
            let blue = Color32::from_rgb(0x3B, 0x82, 0xF6);
            painter.circle_filled(p(12.0, 12.0), 10.0 * s, blue);
            painter.line_segment([p(2.6, 9.5), p(21.4, 9.5)], white);
            painter.line_segment([p(2.6, 14.5), p(21.4, 14.5)], white);
            for cx in [8.6, 15.4] {
                painter.add(CubicBezierShape::from_points_stroke(
                    [p(12.0, 2.2), p(cx, 5.4), p(cx, 18.6), p(12.0, 21.8)],
                    false,
                    Color32::TRANSPARENT,
                    white,
                ));
            }
        }
        TargetType::Folder => {
            let amber = Color32::from_rgb(0xF5, 0x9E, 0x0B);
            let radius = CornerRadius::same((1.5 * s).round() as u8);
            painter.rect_filled(r(2.5, 4.0, 11.4, 9.0), radius, amber);
            painter.rect_filled(r(2.5, 6.4, 21.5, 20.0), radius, amber);
        }
        TargetType::Application => {
            let purple = Color32::from_rgb(0x8B, 0x5C, 0xF6);
            painter.rect_filled(r(4.0, 3.5, 20.0, 20.5), CornerRadius::same((2.0 * s).round() as u8), purple);
            painter.rect_filled(r(7.6, 8.2, 16.4, 10.0), CornerRadius::ZERO, Color32::from_white_alpha(210));
        }
        TargetType::File => {
            let slate = Color32::from_rgb(0x64, 0x74, 0x8B);
            let points =
                vec![p(7.0, 1.5), p(14.0, 1.5), p(19.5, 7.0), p(19.5, 21.0), p(18.0, 22.5), p(7.0, 22.5), p(5.5, 21.0), p(5.5, 3.0)];
            painter.add(egui::Shape::convex_polygon(points, slate, Stroke::NONE));
            painter.line_segment([p(8.6, 12.0), p(16.4, 12.0)], white);
            painter.line_segment([p(8.6, 15.4), p(16.4, 15.4)], white);
            painter.line_segment([p(8.6, 18.8), p(13.6, 18.8)], white);
        }
        TargetType::Unknown => {
            let gray = Color32::from_rgb(0x94, 0xA3, 0xB8);
            painter.circle_filled(p(12.0, 12.0), 10.0 * s, gray);
            painter.text(p(12.0, 12.5), egui::Align2::CENTER_CENTER, "?", style::bold(14.0 * s), Color32::WHITE);
        }
    }
}

// ------------------------------------------------------------------ 入力欄

/// 左に虫眼鏡、空欄のときは薄い説明、文字があれば右に ✕ を出す検索欄（C# 版の検索欄と同じ見た目）。
/// 戻り値は (入力欄の Response, ✕ が押されたか)。
pub fn search_box(
    ui: &mut Ui,
    p: &Palette,
    text: &mut String,
    placeholder: &str,
    id: egui::Id,
    font_size: f32,
    width: f32,
    frame: bool,
) -> (Response, bool) {
    let height = font_size + 18.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if frame {
        ui.painter().rect(rect, CornerRadius::same(6), p.input_background, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    }
    let icon_w = font_size + 14.0;
    ui.painter().text(
        pos2(rect.left() + icon_w / 2.0 + 2.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        "🔍",
        style::regular(font_size * 0.9),
        p.text_secondary,
    );

    let clear_w = if text.is_empty() { 0.0 } else { 28.0 };
    let edit_rect = Rect::from_min_max(pos2(rect.left() + icon_w, rect.top()), pos2(rect.right() - clear_w - 4.0, rect.bottom()));
    let response = ui
        .scope_builder(egui::UiBuilder::new().max_rect(edit_rect), |ui| {
            ui.add_sized(
                edit_rect.size(),
                egui::TextEdit::singleline(text)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .font(style::regular(font_size))
                    .hint_text(egui::RichText::new(placeholder).color(p.text_disabled))
                    .align(egui::Align2::LEFT_CENTER),
            )
        })
        .inner;

    let mut cleared = false;
    if !text.is_empty() {
        let clear_rect = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), Vec2::splat(24.0));
        let r = ui.interact(clear_rect, id.with("clear"), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(clear_rect, CornerRadius::same(5), p.surface_hover);
        }
        ui.painter().text(clear_rect.center(), egui::Align2::CENTER_CENTER, "✕", style::regular(12.0), p.text_secondary);
        if r.clicked() {
            text.clear();
            cleared = true;
        }
        r.on_hover_text(crate::loc::t("Str.Main.ClearSearchTooltip"));
    }
    (response, cleared)
}

/// 枠付きの 1 行入力欄（C# 版の TextBox と同じ高さ・余白）。
pub fn text_field(ui: &mut Ui, text: &mut String, id: egui::Id, width: f32) -> Response {
    let p = current_palette(ui);
    framed_text_edit(
        ui,
        &p,
        vec2(width, 32.0),
        egui::TextEdit::singleline(text).id(id).font(style::regular(13.5)).margin(vec2(8.0, 6.0)).align(egui::Align2::LEFT_CENTER),
    )
}

/// 入力欄を C# 版の TextBox と同じ枠で描く（フォーカスがあるときは枠をアクセント色にする）。
pub fn framed_text_edit(ui: &mut Ui, p: &Palette, size: Vec2, edit: egui::TextEdit) -> Response {
    framed_text_edit_inset(ui, p, size, vec2(9.0, 0.0), edit)
}

/// framed_text_edit の内側の余白を指定できるもの（複数行の入力欄は上下にも余白を取る）。
pub fn framed_text_edit_inset(ui: &mut Ui, p: &Palette, size: Vec2, inset: Vec2, edit: egui::TextEdit) -> Response {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let enabled = ui.is_enabled();
    ui.painter().rect_filled(rect, CornerRadius::same(6), if enabled { p.input_background } else { p.sidebar_background });
    let response = ui.put(rect.shrink2(inset), edit.frame(egui::Frame::NONE));
    let color = if response.has_focus() {
        p.accent
    } else if response.hovered() && enabled {
        p.text_secondary
    } else {
        p.border_strong
    };
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, color), StrokeKind::Inside);
    response
}

/// いまの配色（ダークかどうかは egui の見た目から判断する）。
fn current_palette(ui: &Ui) -> Palette {
    if ui.visuals().dark_mode { style::DARK } else { style::LIGHT }
}

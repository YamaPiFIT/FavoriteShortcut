//! 見た目（配色・文字・角丸）。配色は C# 版の Themes/Light.xaml・Dark.xaml と同じ値。

use std::sync::{Arc, OnceLock};

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// 画面の配色（C# 版のブラシ名に合わせている）。
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub window_background: Color32,
    pub sidebar_background: Color32,
    pub header_background: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub surface_selected: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_disabled: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub accent_foreground: Color32,
    pub input_background: Color32,
    pub tag_background: Color32,
    pub tag_foreground: Color32,
    pub danger: Color32,
    /// C# 版の WarningBrush（今は使っていないが、配色をそろえて持っておく）
    #[allow(dead_code)]
    pub warning: Color32,
    pub dark: bool,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const LIGHT: Palette = Palette {
    window_background: hex(0xF6F7F9),
    sidebar_background: hex(0xEDEFF3),
    header_background: hex(0xFFFFFF),
    surface: hex(0xFFFFFF),
    surface_hover: hex(0xF0F3F8),
    surface_selected: hex(0xE3EBFB),
    border: hex(0xDDE1E8),
    border_strong: hex(0xC4CAD4),
    text_primary: hex(0x1B1F27),
    text_secondary: hex(0x68707E),
    text_disabled: hex(0xA3AAB6),
    accent: hex(0x3B5BDB),
    accent_hover: hex(0x2F4BC4),
    accent_soft: hex(0xE4EAFC),
    accent_foreground: hex(0xFFFFFF),
    input_background: hex(0xFFFFFF),
    tag_background: hex(0xE8ECF5),
    tag_foreground: hex(0x414B5E),
    danger: hex(0xD6336C),
    warning: hex(0xE8590C),
    dark: false,
};

pub const DARK: Palette = Palette {
    window_background: hex(0x16181D),
    sidebar_background: hex(0x1B1E24),
    header_background: hex(0x1F222A),
    surface: hex(0x22262E),
    surface_hover: hex(0x2B303A),
    surface_selected: hex(0x2E3A57),
    border: hex(0x2F343E),
    border_strong: hex(0x414854),
    text_primary: hex(0xE9ECF2),
    text_secondary: hex(0x9AA3B2),
    text_disabled: hex(0x6B7382),
    accent: hex(0x5C7CFA),
    accent_hover: hex(0x748FFC),
    accent_soft: hex(0x2A3350),
    accent_foreground: hex(0xFFFFFF),
    input_background: hex(0x1A1D23),
    tag_background: hex(0x2C323C),
    tag_foreground: hex(0xC3CAD6),
    danger: hex(0xF06595),
    warning: hex(0xFF922B),
    dark: true,
};

/// 太字（C# 版の SemiBold）に使うフォントの名前。
pub const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// 本文の大きさ（C# 版の UiFontSize と同じ 13）。
pub const BODY: f32 = 13.0;
/// 補足の文字（CaptionText）
pub const CAPTION: f32 = 11.5;
/// 見出し（SectionHeaderText）
pub const SECTION: f32 = 11.0;

static FONTS: OnceLock<FontDefinitions> = OnceLock::new();

/// Windows の日本語フォント（Yu Gothic UI）を読み込む。全ウィンドウで共有する。
pub fn fonts() -> FontDefinitions {
    FONTS.get_or_init(build_fonts).clone()
}

fn build_fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let fonts_dir = std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");

    // (ファイル, フォントのファミリー名, 登録名)
    let candidates_regular = [
        ("YuGothR.ttc", "Yu Gothic UI", "yu-gothic-ui"),
        ("meiryo.ttc", "Meiryo UI", "meiryo-ui"),
        ("segoeui.ttf", "Segoe UI", "segoe-ui"),
    ];
    let candidates_bold = [
        ("YuGothM.ttc", "Yu Gothic UI Semibold", "yu-gothic-ui-semibold"),
        ("YuGothB.ttc", "Yu Gothic UI", "yu-gothic-ui-bold"),
        ("meiryob.ttc", "Meiryo UI", "meiryo-ui-bold"),
        ("segoeuib.ttf", "Segoe UI", "segoe-ui-bold"),
    ];
    let symbols = [("seguisym.ttf", "Segoe UI Symbol", "segoe-ui-symbol")];

    let mut regular_names = Vec::new();
    for (file, family, name) in candidates_regular {
        if let Some(data) = load_face(&fonts_dir.join(file), family) {
            defs.font_data.insert(name.into(), Arc::new(data));
            regular_names.push(name.to_string());
        }
    }
    let mut bold_names = Vec::new();
    for (file, family, name) in candidates_bold {
        if let Some(data) = load_face(&fonts_dir.join(file), family) {
            defs.font_data.insert(name.into(), Arc::new(data));
            bold_names.push(name.to_string());
            break; // 太字は 1 つあれば十分（足りない文字は通常の文字へ流れる）
        }
    }
    let mut symbol_names = Vec::new();
    for (file, family, name) in symbols {
        if let Some(data) = load_face(&fonts_dir.join(file), family) {
            defs.font_data.insert(name.into(), Arc::new(data));
            symbol_names.push(name.to_string());
        }
    }

    // 通常の文字: Yu Gothic UI → Meiryo UI → Segoe UI → 記号 → egui 既定（絵文字など）
    let proportional = defs.families.entry(FontFamily::Proportional).or_default();
    let fallback: Vec<String> = proportional.clone();
    let mut list = regular_names.clone();
    list.extend(symbol_names.iter().cloned());
    list.extend(fallback.iter().cloned());
    *proportional = list;

    // 太字: 太字のフォント → 通常の並び
    let mut bold_list = bold_names;
    bold_list.extend(regular_names.iter().cloned());
    bold_list.extend(symbol_names.iter().cloned());
    bold_list.extend(fallback.iter().cloned());
    defs.families.insert(FontFamily::Name(BOLD.into()), bold_list);

    // 等幅にも日本語を足しておく（パスの表示などで化けないように）
    let mono = defs.families.entry(FontFamily::Monospace).or_default();
    mono.extend(regular_names);
    defs
}

/// フォントファイル（.ttc はコレクション）から、指定したファミリー名の書体を取り出す。
///
/// 日本語のフォントは 1 ファイル 10MB 以上あるので、丸ごと読み込まずにメモリマップで参照する
/// （使う部分だけを Windows が読み、アプリのメモリとしては数えられない）。
/// 開けなかったときだけ、ファイルを読み込んで使う。
fn load_face(path: &std::path::Path, family: &str) -> Option<FontData> {
    let blob: egui::epaint::text::Blob = match crate::platform::MappedFile::open(path) {
        Some(file) => Arc::new(file),
        None => Arc::new(std::fs::read(path).ok()?),
    };
    let index = face_index(blob.as_ref().as_ref(), family)?;
    Some(FontData::from_blob(blob, index))
}

/// 指定したファミリー名の書体が、ファイルの何番目にあるか。
fn face_index(bytes: &[u8], family: &str) -> Option<u32> {
    let count = ttf_parser::fonts_in_collection(bytes).unwrap_or(1);
    let mut chosen = None;
    for index in 0..count {
        let Ok(face) = ttf_parser::Face::parse(bytes, index) else { continue };
        let names: Vec<String> = face
            .names()
            .into_iter()
            .filter(|n| {
                n.name_id == ttf_parser::name_id::FAMILY
                    || n.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                    || n.name_id == ttf_parser::name_id::FULL_NAME
            })
            .filter_map(|n| n.to_string())
            .collect();
        if names.iter().any(|n| n.eq_ignore_ascii_case(family)) {
            chosen = Some(index);
            break;
        }
    }
    match chosen {
        Some(i) => Some(i),
        None if count == 1 => Some(0),
        None => None,
    }
}

/// egui の見た目を配色に合わせる。
pub fn apply(ctx: &egui::Context, p: &Palette) {
    ctx.set_fonts(fonts());

    let mut visuals = if p.dark { Visuals::dark() } else { Visuals::light() };
    visuals.panel_fill = p.window_background;
    visuals.window_fill = p.surface;
    visuals.extreme_bg_color = p.input_background;
    visuals.faint_bg_color = p.surface_hover;
    visuals.code_bg_color = p.sidebar_background;
    visuals.override_text_color = Some(p.text_primary);
    visuals.hyperlink_color = p.accent;
    visuals.selection.bg_fill = p.accent.gamma_multiply(if p.dark { 0.55 } else { 0.35 });
    // 選択した文字の色（入力欄の枠のアクセント色は widgets::framed_text_edit で描く）
    visuals.selection.stroke = Stroke::new(1.0, p.text_primary);
    visuals.window_stroke = Stroke::new(1.0, p.border_strong);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.text_edit_bg_color = Some(p.input_background);

    let r = CornerRadius::same(6);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_primary);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.surface;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text_primary);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = p.surface_hover;
    w.hovered.weak_bg_fill = p.surface_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text_primary);
    w.hovered.corner_radius = r;

    w.active.bg_fill = p.surface_selected;
    w.active.weak_bg_fill = p.surface_selected;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text_primary);
    w.active.corner_radius = r;

    w.open.bg_fill = p.surface;
    w.open.weak_bg_fill = p.surface;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text_primary);
    w.open.corner_radius = r;

    // egui はダーク用とライト用の見た目を別々に持ち、OS の設定に合わせて切り替える。
    // テーマはアプリ側で決めるので固定し、両方に同じ見た目を入れておく
    ctx.set_theme(if p.dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Light, visuals);

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, regular(CAPTION)),
            (TextStyle::Body, regular(BODY)),
            (TextStyle::Button, regular(BODY)),
            (TextStyle::Heading, bold(14.5)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(14.0, 6.0);
        style.spacing.interact_size = egui::vec2(28.0, 30.0);
        style.spacing.combo_height = 300.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating = true;
        style.interaction.selectable_labels = false;
        style.url_in_tooltip = false;
    });
}

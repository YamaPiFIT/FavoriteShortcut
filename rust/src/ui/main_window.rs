//! 管理画面（C# 版 MainWindow と同じ構成）。
//!
//!   上: アプリ名・検索欄・表示切替（カード / リスト）・新規登録・メニュー
//!   左: フォルダのツリー（ドラッグで移動できる）・タグ
//!   右: ショートカットの一覧（見えている分だけ描くので、件数が多くても重くならない）
//!   下: 件数・ホットキーの案内

use std::collections::HashSet;
use std::time::{Duration, Instant};

use egui::{Align, Color32, CornerRadius, Key, Layout, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::app::{Command, Core, Owner};
use crate::loc;
use crate::model::{Shortcut, ViewMode};
use crate::normalize::split_terms;
use crate::ui::dialogs::TextPurpose;
use crate::ui::style::{self, Palette};
use crate::ui::widgets::{self, ButtonKind};
use crate::util::culture_cmp_ignore_case;

/// カード表示の表示件数の上限（打ち切った場合は画面で知らせる。C# 版と同じ）。
const MAX_CARD_ITEMS: usize = 1000;
const CARD_W: f32 = 180.0;
const CARD_H: f32 = 132.0;
const CARD_MARGIN: f32 = 4.0;
/// 検索欄に入力してから一覧を作り直すまでの待ち時間（打ち終わるのを少し待つ）
const SEARCH_DELAY: Duration = Duration::from_millis(140);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    All,
    Uncategorized,
    Folder(String),
}

#[derive(Clone)]
struct ShortcutDrag(Vec<String>);

#[derive(Clone)]
struct FolderDrag(String);

pub struct MainState {
    scope: Scope,
    search: String,
    applied_query: String,
    search_changed_at: Option<Instant>,
    displayed: Vec<String>,
    truncated: bool,
    scope_count: usize,
    list_key: Option<(u64, String, Scope, ViewMode)>,
    shown_query: Option<(String, Scope)>,
    selected: HashSet<String>,
    anchor: Option<usize>,
    cursor: Option<usize>,
    collapsed: HashSet<String>,
    scroll_to_top: bool,
    scroll_to_cursor: bool,
    focus_search: bool,
    select_all_search: bool,
    focus_list: bool,
    pane_width: f32,
    tag_cache: Option<(u64, Vec<(String, usize)>)>,
    columns: usize,
    menu_open: bool,
    /// 前のフレームの各欄の位置（パネルの大きさは前のフレームの中身で決まるため、変わったら描き直して合わせる）
    layout_signature: Vec<egui::Rect>,
    /// 外（エクスプローラーやブラウザ）からドラッグしているものの位置と、それがファイルか
    external_drag: Option<(egui::Pos2, bool)>,
    /// フォルダツリーの各行の位置とフォルダ（前のフレーム。外からのドロップ先を決めるため）
    drop_rows: Vec<(egui::Rect, Option<String>)>,
    /// フォルダツリーの欄の位置（前のフレーム）
    tree_area: egui::Rect,
}

impl MainState {
    pub fn new(core: &Core) -> Self {
        let s = &core.settings;
        let mut scope = Scope::All;
        if s.restore_last_folder
            && let Some(id) = &s.last_folder_id
            && core.store.folder(id).is_some()
        {
            scope = Scope::Folder(id.clone());
        }
        Self {
            scope,
            search: String::new(),
            applied_query: String::new(),
            search_changed_at: None,
            displayed: Vec::new(),
            truncated: false,
            scope_count: 0,
            list_key: None,
            shown_query: None,
            selected: HashSet::new(),
            anchor: None,
            cursor: None,
            collapsed: HashSet::new(),
            scroll_to_top: false,
            scroll_to_cursor: false,
            focus_search: true,
            select_all_search: false,
            focus_list: false,
            pane_width: (s.folder_pane_width as f32).clamp(170.0, 460.0),
            tag_cache: None,
            columns: 1,
            menu_open: false,
            layout_signature: Vec::new(),
            external_drag: None,
            drop_rows: Vec::new(),
            tree_area: egui::Rect::NOTHING,
        }
    }

    /// 表示されたとき（C# 版の Loaded と同じく、検索欄にフォーカスを置く）。
    pub fn on_shown(&mut self) {
        self.focus_search = true;
    }

    pub fn current_folder_id(&self) -> Option<String> {
        match &self.scope {
            Scope::Folder(id) => Some(id.clone()),
            _ => None,
        }
    }

    /// 外からドラッグしているものの位置（ウィンドウ内の座標）。None ならドラッグしていない。
    pub fn set_external_drag(&mut self, drag: Option<(egui::Pos2, bool)>) {
        self.external_drag = drag;
    }

    /// 外からドロップされたものの登録先。None なら受け付けない。
    /// C# 版と同じく、ファイルはツリーのフォルダへ落とせばそのフォルダ（「すべて」「未分類」や
    /// ツリーの空いている所なら未分類）、それ以外の場所なら今表示しているフォルダに登録する。
    /// 文字列（URL など）はツリーには落とせない。
    pub fn drop_destination(&self, pos: Option<egui::Pos2>, files: bool) -> Option<Option<String>> {
        if let Some(pos) = pos {
            if let Some((_, folder)) = self.drop_rows.iter().find(|(r, _)| r.contains(pos)) {
                return files.then(|| folder.clone());
            }
            if self.tree_area.contains(pos) {
                return files.then_some(None);
            }
        }
        Some(self.current_folder_id())
    }

    /// フォルダを選ぶ（None なら「すべて」）。
    pub fn select_folder(&mut self, id: Option<String>) {
        self.scope = id.map_or(Scope::All, Scope::Folder);
        self.list_key = None;
    }

    /// ウィンドウの大きさ・フォルダ欄の幅・選んでいたフォルダを保存する。
    pub fn persist(&mut self, core: &mut Core, window: &winit::window::Window) {
        let mut s = core.settings.clone();
        if !window.is_maximized() && window.is_minimized() != Some(true) {
            let size = window.inner_size().to_logical::<f64>(window.scale_factor());
            if size.width > 100.0 && size.height > 100.0 {
                s.main_window_width = size.width;
                s.main_window_height = size.height;
            }
        }
        s.folder_pane_width = self.pane_width as f64;
        s.last_folder_id = self.current_folder_id();
        if s != core.settings {
            core.save_settings(s);
        }
    }

    fn selected_ids(&self) -> Vec<String> {
        // 表示順に並べる
        self.displayed.iter().filter(|id| self.selected.contains(*id)).cloned().collect()
    }

    fn select_only(&mut self, index: usize) {
        if let Some(id) = self.displayed.get(index) {
            self.selected.clear();
            self.selected.insert(id.clone());
            self.anchor = Some(index);
            self.cursor = Some(index);
        }
    }

    fn select_range(&mut self, index: usize) {
        let anchor = self.anchor.unwrap_or(index);
        let (a, b) = if anchor <= index { (anchor, index) } else { (index, anchor) };
        self.selected = self.displayed[a..=b.min(self.displayed.len().saturating_sub(1))].iter().cloned().collect();
        self.cursor = Some(index);
    }

    // ---------------------------------------------------------------- 一覧

    fn refresh_list(&mut self, core: &mut Core) {
        let view = core.settings.view_mode;
        let key = (core.store.data_version, self.applied_query.clone(), self.scope.clone(), view);
        if self.list_key.as_ref() == Some(&key) {
            return;
        }
        // 選んでいたフォルダが消えていたら「すべて」に戻す
        if let Scope::Folder(id) = &self.scope
            && core.store.folder(id).is_none()
        {
            self.scope = Scope::All;
        }

        let source: Vec<&Shortcut> = match &self.scope {
            Scope::Folder(id) => {
                // サブフォルダの中身も含めて表示する（親フォルダを選ぶと全体が見える）
                let ids: HashSet<String> = core.store.collect_folder_and_descendants(id).into_iter().collect();
                core.store.shortcuts().iter().filter(|s| s.folder_id.as_ref().is_some_and(|f| ids.contains(f))).collect()
            }
            Scope::Uncategorized => core.store.shortcuts().iter().filter(|s| s.folder_id.is_none()).collect(),
            Scope::All => core.store.shortcuts().iter().collect(),
        };
        self.scope_count = source.len();

        let revision = core.store.revision;
        let mut result: Vec<String> =
            match crate::search::search(source.iter().copied(), &self.applied_query, &mut core.search, revision, usize::MAX) {
                Some(hits) => hits.into_iter().map(|h| h.id).collect(),
                None => {
                    let mut list = source.clone();
                    list.sort_by(|a, b| {
                        a.sort_order.cmp(&b.sort_order).then_with(|| culture_cmp_ignore_case(a.display_title(), b.display_title()))
                    });
                    list.into_iter().map(|s| s.id.clone()).collect()
                }
            };

        self.truncated = view == ViewMode::Card && result.len() > MAX_CARD_ITEMS;
        if self.truncated {
            result.truncate(MAX_CARD_ITEMS);
        }

        // 検索語や表示するフォルダが変わったときは、一覧を先頭から表示する
        let shown = (split_terms(&self.applied_query).join(" "), self.scope.clone());
        if self.shown_query.as_ref().is_some_and(|s| *s != shown) {
            self.scroll_to_top = true;
        }
        self.shown_query = Some(shown);

        let still: HashSet<&String> = result.iter().collect();
        self.selected.retain(|id| still.contains(id));
        if self.cursor.is_some_and(|c| c >= result.len()) {
            self.cursor = None;
            self.anchor = None;
        }
        self.displayed = result;
        self.list_key = Some(key);
        core.status_override = None;
    }
}

// -------------------------------------------------------------------- 描画

pub fn ui(ui: &mut Ui, st: &mut MainState, core: &mut Core) {
    let p = core.palette;

    // 検索語は打ち終わるのを少し待ってから反映する
    if let Some(t) = st.search_changed_at {
        if t.elapsed() >= SEARCH_DELAY {
            st.search_changed_at = None;
            st.applied_query = st.search.clone();
        } else {
            ui.ctx().request_repaint_after(SEARCH_DELAY - t.elapsed());
        }
    }
    st.refresh_list(core);

    handle_window_shortcuts(ui, st, core);

    let header_panel = egui::Panel::top("header")
        .frame(egui::Frame::new().fill(p.header_background).inner_margin(egui::Margin::symmetric(14, 10)).stroke(Stroke::NONE))
        .show_separator_line(false)
        .show(ui, |ui| header(ui, st, core, &p));
    hline(ui, &p);

    let status_panel = egui::Panel::bottom("status")
        .frame(egui::Frame::new().fill(p.header_background).inner_margin(egui::Margin::symmetric(14, 6)))
        .show_separator_line(false)
        .show(ui, |ui| status_bar(ui, st, core, &p));

    let side = egui::Panel::left("folders")
        .resizable(true)
        .default_size(st.pane_width)
        .size_range(170.0..=460.0)
        .frame(egui::Frame::new().fill(p.sidebar_background))
        .show_separator_line(false)
        .show(ui, |ui| sidebar(ui, st, core, &p));
    st.pane_width = side.response.rect.width();

    egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(p.window_background)).show(ui, |ui| content(ui, st, core, &p));

    // パネルの大きさは前のフレームの中身で決まるので、変わったらもう一度描いて合わせる
    let signature = vec![header_panel.response.rect, status_panel.response.rect, side.response.rect, side.inner];
    if signature != st.layout_signature {
        st.layout_signature = signature;
        ui.ctx().request_repaint();
    }
}

fn hline(ui: &mut Ui, p: &Palette) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, p.border));
}

fn header(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette) {
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(widgets::rich(&loc::t("Str.App.Name"), style::bold(14.5), p.text_primary));
        ui.add_space(14.0);

        // 右側のボタンの幅を残して、検索欄は最大 560 まで広げる
        let right_width = 330.0;
        let width = (ui.available_width() - right_width).clamp(160.0, 560.0);
        let id = egui::Id::unique("main-search");
        let before = st.search.clone();
        let (response, _cleared) = widgets::search_box(ui, p, &mut st.search, &loc::t("Str.Main.SearchPlaceholder"), id, 13.5, width, true);
        if st.search != before {
            st.search_changed_at = Some(Instant::now());
            ui.ctx().request_repaint_after(SEARCH_DELAY);
        }
        if st.focus_search {
            st.focus_search = false;
            response.request_focus();
        }
        if st.select_all_search {
            st.select_all_search = false;
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let range = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(st.search.chars().count()));
                state.cursor.set_char_range(Some(range));
                state.store(ui.ctx(), id);
            }
        }
        if response.has_focus() {
            search_keys(ui, st, core);
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let menu_button =
                widgets::button_ex(ui, p, ButtonKind::Secondary, "⋯", true, Some(38.0)).on_hover_text(loc::t("Str.Main.MoreTooltip"));
            app_menu(ui, &menu_button, st, core);
            ui.add_space(8.0);
            if widgets::button(ui, p, ButtonKind::Primary, &loc::t("Str.Main.New")).on_hover_text(loc::t("Str.Main.NewTooltip")).clicked() {
                core.push(Command::OpenEdit { existing: None, folder: st.current_folder_id(), initial: None, owner: Owner::Main });
            }
            ui.add_space(10.0);
            // 表示切替（カード / リスト）
            egui::Frame::new().fill(p.sidebar_background).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::same(2)).show(
                ui,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let mode = core.settings.view_mode;
                    let list = widgets::toggle(ui, p, &loc::t("Str.Main.ListView"), mode == ViewMode::List)
                        .on_hover_text(loc::t("Str.Main.ListViewTooltip"));
                    let card = widgets::toggle(ui, p, &loc::t("Str.Main.CardView"), mode == ViewMode::Card)
                        .on_hover_text(loc::t("Str.Main.CardViewTooltip"));
                    let new_mode = if card.clicked() {
                        Some(ViewMode::Card)
                    } else if list.clicked() {
                        Some(ViewMode::List)
                    } else {
                        None
                    };
                    if let Some(m) = new_mode
                        && m != mode
                    {
                        let mut s = core.settings.clone();
                        s.view_mode = m;
                        core.save_settings(s);
                        st.list_key = None;
                    }
                },
            );
        });
    });
}

/// 「⋯」のメニュー（C# 版と同じ並び）。
fn app_menu(_ui: &mut Ui, button: &egui::Response, st: &mut MainState, core: &mut Core) {
    egui::Popup::menu(button).show(|ui| {
        ui.set_min_width(220.0);
        let item = |ui: &mut Ui, key: &str| ui.button(loc::t(key)).clicked();
        if item(ui, "Str.Menu.Settings") {
            core.push(Command::OpenSettings);
        }
        ui.separator();
        if item(ui, "Str.Menu.ImportBookmarks") {
            core.push(Command::OpenBookmarkImport { folder: st.current_folder_id() });
        }
        ui.separator();
        if item(ui, "Str.Menu.Export") {
            core.push(Command::Export);
        }
        if item(ui, "Str.Menu.Import") {
            core.push(Command::Import);
        }
        ui.separator();
        if item(ui, "Str.Menu.BackupNow") {
            core.push(Command::BackupNow);
        }
        if item(ui, "Str.Menu.OpenDataFolder") {
            core.push(Command::OpenDataFolder);
        }
        ui.separator();
        if item(ui, "Str.Menu.ShowLauncher") {
            core.push(Command::ShowLauncher);
        }
        if item(ui, "Str.Menu.Help") {
            core.push(Command::ShowHelp);
        }
    });
    st.menu_open = false;
}

fn status_bar(ui: &mut Ui, _st: &mut MainState, core: &mut Core, p: &Palette) {
    ui.horizontal(|ui| {
        let text = core.status_override.clone().unwrap_or_else(|| {
            loc::tf("Str.Main.Status", &[&core.store.shortcut_count(), &core.store.folder_count(), &core.store.tag_count()])
        });
        ui.label(widgets::rich(&text, style::regular(style::CAPTION), p.text_secondary));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let hint = match core.hotkeys.as_ref().and_then(|h| h.launcher_hotkey()) {
                Some((m, k)) => loc::tf("Str.Main.HotKeyHint", &[&crate::keys::describe(m, k)]),
                None => loc::t("Str.Main.HotKeyNotRegistered"),
            };
            ui.label(widgets::rich(&hint, style::regular(style::CAPTION), p.text_secondary));
        });
    });
}

// ------------------------------------------------------------ フォルダとタグ

/// フォルダとタグの欄。タグ欄の位置を返す（大きさの変化を確かめるため）。
fn sidebar(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette) -> egui::Rect {
    // 下のタグ欄（高さは中身次第、最大 150 + 見出し）
    let tags_rect = egui::Panel::bottom("tags")
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 10, top: 10, bottom: 12 }))
        .show_separator_line(true)
        .show(ui, |ui| tag_cloud(ui, st, core, p))
        .response
        .rect;

    egui::CentralPanel::no_frame().show(ui, |ui| {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            widgets::section_header(ui, p, &loc::t("Str.Folder.Section"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(10.0);
                if widgets::button(ui, p, ButtonKind::Ghost, "＋").on_hover_text(loc::t("Str.Folder.NewTooltip")).clicked() {
                    open_new_folder(core, None);
                }
            });
        });
        ui.add_space(4.0);
        // ドラッグ中の位置（アプリ内のドラッグ、または外からのファイルのドラッグ）
        let drag_pos = if egui::DragAndDrop::has_any_payload(ui.ctx()) {
            ui.ctx().pointer_latest_pos()
        } else {
            st.external_drag.filter(|(_, files)| *files).map(|(pos, _)| pos)
        };
        egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]).show(ui, |ui| {
            let viewport = ui.clip_rect();
            st.tree_area = viewport;
            // ドラッグ中に上端・下端へ近づけたら、その方向へスクロールする（画面外のフォルダへも移せるように）
            if let Some(pos) = drag_pos
                && viewport.x_range().contains(pos.x)
            {
                const EDGE: f32 = 24.0;
                if pos.y < viewport.top() + EDGE && pos.y >= viewport.top() - EDGE {
                    ui.scroll_with_delta(vec2(0.0, 6.0));
                    ui.ctx().request_repaint();
                } else if pos.y > viewport.bottom() - EDGE && pos.y <= viewport.bottom() + EDGE {
                    ui.scroll_with_delta(vec2(0.0, -6.0));
                    ui.ctx().request_repaint();
                }
            }
            ui.add_space(2.0);
            folder_tree(ui, st, core, p);
            ui.add_space(8.0);
        });
    });
    tags_rect
}

fn open_new_folder(core: &mut Core, parent: Option<String>) {
    core.push(Command::OpenTextInput {
        purpose: TextPurpose::NewFolder { parent },
        title: loc::t("Str.Folder.NewTitle"),
        label: loc::t("Str.Folder.NameLabel"),
        initial: String::new(),
    });
}

fn folder_tree(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette) {
    st.drop_rows.clear();
    tree_row(ui, st, core, p, TreeNode::All, 0);
    tree_row(ui, st, core, p, TreeNode::Uncategorized, 0);
    let roots = core.store.root_folders().to_vec();
    for id in roots {
        folder_subtree(ui, st, core, p, &id, 0);
    }

    // 選択中のフォルダの名前変更（F2）。一覧や入力欄にフォーカスがあるときは、そちらの操作を優先する
    let list_focused = ui.memory(|m| m.has_focus(egui::Id::unique("shortcut-list")));
    if ui.input(|i| i.key_pressed(Key::F2))
        && !ui.ctx().text_edit_focused()
        && !list_focused
        && !st.focus_list
        && let Scope::Folder(id) = &st.scope
    {
        rename_folder(core, id.clone());
    }
}

fn folder_subtree(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette, id: &str, depth: usize) {
    tree_row(ui, st, core, p, TreeNode::Folder(id.to_string()), depth);
    if st.collapsed.contains(id) {
        return;
    }
    let children = core.store.folder(id).map(|f| f.children.clone()).unwrap_or_default();
    for child in children {
        folder_subtree(ui, st, core, p, &child, depth + 1);
    }
}

enum TreeNode {
    All,
    Uncategorized,
    Folder(String),
}

fn tree_row(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette, node: TreeNode, depth: usize) {
    let (scope, glyph, name, bold) = match &node {
        TreeNode::All => (Scope::All, "★", loc::t("Str.Folder.All"), true),
        TreeNode::Uncategorized => (Scope::Uncategorized, "◇", loc::t("Str.Folder.Uncategorized"), true),
        TreeNode::Folder(id) => {
            let name = core.store.folder(id).map(|f| f.name.clone()).unwrap_or_default();
            (Scope::Folder(id.clone()), "📁", name, false)
        }
    };
    let folder_id = match &node {
        TreeNode::Folder(id) => Some(id.clone()),
        _ => None,
    };
    let has_children = folder_id.as_ref().is_some_and(|id| core.store.folder(id).is_some_and(|f| !f.children.is_empty()));

    let width = ui.available_width() - 4.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click_and_drag());
    let rect = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top()), rect.max);
    let selected = st.scope == scope;

    // 外からのドロップ先を決めるため、見えている部分の位置を覚えておく
    let visible = rect.intersect(ui.clip_rect());
    if visible.is_positive() {
        st.drop_rows.push((visible, folder_id.clone()));
    }

    // ドラッグ中のドロップ先の判定
    let shortcut_hover = response.dnd_hover_payload::<ShortcutDrag>().is_some();
    // フォルダは自分自身や、自分の配下へは移せない（「すべて」「未分類」へ落とすと最上位へ移る。C# 版と同じ）
    let folder_hover = response
        .dnd_hover_payload::<FolderDrag>()
        .is_some_and(|d| folder_id.as_ref().is_none_or(|target| !core.store.is_descendant_of(target, &d.0)));
    // 外から（エクスプローラーから）ファイルをドラッグしているとき
    let external_hover = st.external_drag.is_some_and(|(pos, files)| files && visible.contains(pos));
    let drop_target = shortcut_hover || folder_hover || external_hover;

    let fill = if drop_target {
        p.accent
    } else if selected {
        p.surface_selected
    } else if response.hovered() {
        p.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let fg = if drop_target { Color32::WHITE } else { p.text_primary };

    let indent = 14.0 * depth as f32;
    let arrow_x = rect.left() + 4.0 + indent;
    if has_children && let Some(id) = &folder_id {
        let arrow_rect = Rect::from_min_size(pos2(arrow_x, rect.center().y - 8.0), vec2(16.0, 16.0));
        let arrow = ui.interact(arrow_rect, ui.scope_id().with(("arrow", id)), Sense::click());
        let c = rect.center().y;
        let x = arrow_rect.center().x;
        let stroke = Stroke::new(1.5, if drop_target { Color32::WHITE } else { p.text_secondary });
        if st.collapsed.contains(id) {
            ui.painter().line(vec![pos2(x - 2.0, c - 4.0), pos2(x + 2.0, c), pos2(x - 2.0, c + 4.0)], stroke);
        } else {
            ui.painter().line(vec![pos2(x - 4.0, c - 2.0), pos2(x, c + 2.5), pos2(x + 4.0, c - 2.0)], stroke);
        }
        if arrow.clicked() {
            if !st.collapsed.remove(id) {
                st.collapsed.insert(id.clone());
            }
        }
    }
    let text_x = arrow_x + 16.0;
    ui.painter().text(pos2(text_x, rect.center().y), egui::Align2::LEFT_CENTER, glyph, style::regular(13.0), fg);
    let font = if bold { style::bold(style::BODY) } else { style::regular(style::BODY) };
    let max_w = rect.right() - (text_x + 20.0) - 6.0;
    let galley = ui.painter().layout(name.clone(), font, fg, f32::INFINITY);
    let galley = if galley.size().x > max_w {
        let mut job =
            egui::text::LayoutJob::simple_singleline(name, if bold { style::bold(style::BODY) } else { style::regular(style::BODY) }, fg);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(10.0));
        ui.painter().layout_job(job)
    } else {
        galley
    };
    ui.painter().galley(pos2(text_x + 20.0, rect.center().y - galley.size().y / 2.0), galley, fg);

    if response.clicked() || response.secondary_clicked() {
        if st.scope != scope {
            st.scope = scope.clone();
            st.list_key = None;
        }
        st.selected.clear();
        st.focus_list = false;
    }

    // フォルダをドラッグして別のフォルダへ移す
    if let Some(id) = &folder_id
        && response.drag_started()
    {
        response.dnd_set_drag_payload(FolderDrag(id.clone()));
    }

    // ドロップ
    if let Some(payload) = response.dnd_release_payload::<ShortcutDrag>() {
        core.store.begin_batch();
        for id in &payload.0 {
            if let Err(e) = core.store.move_shortcut(id, folder_id.as_deref()) {
                crate::log::warn_err("移動に失敗しました。", &e);
            }
        }
        core.store.end_batch();
    }
    if let Some(payload) = response.dnd_release_payload::<FolderDrag>() {
        match core.store.move_folder(&payload.0, folder_id.as_deref()) {
            Ok(true) => {}
            Ok(false) => core.push(Command::Message {
                text: loc::t("Str.Folder.CannotMoveIntoSelf"),
                title: loc::t("Str.App.Name"),
                icon: crate::platform::MsgIcon::Info,
            }),
            Err(e) => crate::log::warn_err("フォルダを移動できませんでした。", &e),
        }
    }

    // 右クリックのメニュー
    let is_real = folder_id.is_some();
    response.context_menu(|ui| {
        ui.set_min_width(200.0);
        if ui.button(loc::t("Str.Folder.NewTitle")).clicked() {
            open_new_folder(core, None);
            ui.close();
        }
        if ui.add_enabled(is_real, egui::Button::new(loc::t("Str.Folder.NewSub"))).clicked() {
            open_new_folder(core, folder_id.clone());
            ui.close();
        }
        ui.separator();
        if ui.add_enabled(is_real, egui::Button::new(loc::t("Str.Common.Rename")).shortcut_text("F2")).clicked() {
            if let Some(id) = &folder_id {
                rename_folder(core, id.clone());
            }
            ui.close();
        }
        if ui.add_enabled(is_real, egui::Button::new(loc::t("Str.Common.Delete"))).clicked() {
            if let Some(id) = &folder_id {
                core.push(Command::OpenFolderDelete { folder: id.clone() });
            }
            ui.close();
        }
    });
}

fn rename_folder(core: &mut Core, id: String) {
    let name = core.store.folder(&id).map(|f| f.name.clone()).unwrap_or_default();
    core.push(Command::OpenTextInput {
        purpose: TextPurpose::RenameFolder { id },
        title: loc::t("Str.Folder.RenameTitle"),
        label: loc::t("Str.Folder.NameLabel"),
        initial: name,
    });
}

fn tag_cloud(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette) {
    ui.horizontal(|ui| {
        widgets::section_header(ui, p, &loc::t("Str.Tag.Section"));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = widgets::button(ui, p, ButtonKind::Ghost, &loc::t("Str.Tag.Manage")).on_hover_text(loc::t("Str.Tag.ManageTooltip"));
            if r.clicked() {
                core.push(Command::OpenTagManager);
            }
        });
    });
    ui.add_space(4.0);

    let version = core.store.data_version;
    if st.tag_cache.as_ref().is_none_or(|(v, _)| *v != version) {
        st.tag_cache = Some((version, core.store.tag_usage().into_iter().take(60).collect()));
    }
    let tags = st.tag_cache.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
    if tags.is_empty() {
        widgets::caption(ui, p, &loc::t("Str.Tag.None"));
        return;
    }
    // 並べたときの高さを先に求めておく（スクロール欄は前のフレームの大きさから広がれないため）
    let width = ui.available_width();
    let mut x = 0.0f32;
    let mut rows = 1;
    let mut row_height = 0.0f32;
    for (name, count) in &tags {
        let galley = ui.painter().layout_no_wrap(format!("{name}  {count}"), style::regular(style::CAPTION), p.text_primary);
        let w = galley.size().x + 18.0;
        row_height = row_height.max(galley.size().y + 8.0);
        if x > 0.0 && x + w > width {
            rows += 1;
            x = 0.0;
        }
        x += w + 5.0;
    }
    let needed = rows as f32 * row_height + (rows - 1) as f32 * 5.0;
    egui::ScrollArea::vertical()
        .id_salt("tagcloud")
        .min_scrolled_height(needed.min(150.0))
        .max_height(150.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            // 行の高さをチップの高さにそろえる（折り返しの行の高さは外側の設定で決まり、既定の 30 だと計算とずれる）
            ui.spacing_mut().interact_size.y = row_height;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                for (name, count) in tags {
                    let text = format!("{name}  {count}");
                    let galley = ui.painter().layout_no_wrap(text, style::regular(style::CAPTION), p.text_primary);
                    let size = galley.size() + vec2(18.0, 8.0);
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    let fill = if response.hovered() { p.surface_hover } else { p.surface };
                    ui.painter().rect(rect, CornerRadius::same(6), fill, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
                    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, p.text_primary);
                    let response =
                        response.on_hover_text(loc::tf("Str.Tag.SearchTooltip", &[&name])).on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        st.search = name.clone();
                        st.applied_query = name.clone();
                        st.search_changed_at = None;
                        st.focus_search = true;
                    }
                }
            });
        });
}

// ------------------------------------------------------------------ 一覧

fn content(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        let scope_text = match &st.scope {
            Scope::Folder(id) => core.store.folder(id).map(|f| f.full_path.clone()).unwrap_or_default(),
            Scope::Uncategorized => loc::t("Str.Folder.Uncategorized"),
            Scope::All => loc::t("Str.Folder.All"),
        };
        let searching = !st.applied_query.trim().is_empty();
        let count = if searching {
            loc::tf("Str.Main.CountSearching", &[&st.displayed.len(), &st.scope_count])
        } else {
            loc::tf("Str.Main.Count", &[&st.displayed.len()])
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(18.0);
            ui.label(widgets::rich(&count, style::regular(style::CAPTION), p.text_secondary));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                widgets::truncated(ui, widgets::rich(&scope_text, style::bold(style::BODY), p.text_primary));
            });
        });
    });
    ui.add_space(6.0);

    if st.truncated {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            egui::Frame::new().fill(p.accent_soft).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(10, 7)).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width() - 18.0);
                    ui.label(widgets::rich(
                        &loc::tf("Str.Main.Truncated", &[&MAX_CARD_ITEMS]),
                        style::regular(style::CAPTION),
                        p.text_secondary,
                    ));
                },
            );
        });
        ui.add_space(6.0);
    }

    if st.displayed.is_empty() {
        empty_state(ui, st, core, p);
        return;
    }

    let list_id = egui::Id::unique("shortcut-list");
    // 一覧全体を 1 つのキーボード操作の対象にする（↑↓・Enter・Delete など）
    let list_rect = ui.available_rect_before_wrap();
    let list_response = ui.interact(list_rect, list_id, Sense::focusable_noninteractive());
    if st.focus_list {
        st.focus_list = false;
        list_response.request_focus();
    }
    let has_focus = list_response.has_focus();
    if has_focus {
        list_keys(ui, st, core);
    }

    match core.settings.view_mode {
        ViewMode::List => list_view(ui, st, core, p, list_id),
        ViewMode::Card => card_view(ui, st, core, p, list_id),
    }
}

fn empty_state(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette) {
    let searching = !st.applied_query.trim().is_empty();
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 70.0).max(20.0));
        ui.set_max_width(420.0);
        let title = loc::t(if searching { "Str.Main.NotFoundTitle" } else { "Str.Main.EmptyTitle" });
        let hint = loc::t(if searching { "Str.Main.NotFoundHint" } else { "Str.Main.EmptyHint" });
        ui.label(widgets::rich(&title, style::bold(15.0), p.text_primary));
        ui.add_space(8.0);
        ui.label(widgets::rich(&hint, style::regular(style::CAPTION), p.text_secondary));
        ui.add_space(16.0);
        if widgets::button(ui, p, ButtonKind::Primary, &loc::t("Str.Main.New")).clicked() {
            core.push(Command::OpenEdit { existing: None, folder: st.current_folder_id(), initial: None, owner: Owner::Main });
        }
    });
}

fn row_height(icon: f32) -> f32 {
    icon.max(34.0) + 16.0 + 2.0
}

fn list_view(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette, list_id: egui::Id) {
    let icon = core.settings.icon_size as f32;
    let row_h = row_height(icon) + CARD_MARGIN * 2.0;
    let total = st.displayed.len();
    let mut area = egui::ScrollArea::vertical().id_salt(list_id.with("scroll")).auto_shrink([false, false]);
    if std::mem::take(&mut st.scroll_to_top) {
        area = area.vertical_scroll_offset(0.0);
    }
    if std::mem::take(&mut st.scroll_to_cursor)
        && let Some(c) = st.cursor
    {
        let state = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(list_id.with("scroll")));
        let view_h = ui.available_height();
        let current = state.map(|s| s.clamped_offset().y).unwrap_or(0.0);
        let top = c as f32 * row_h;
        if top < current {
            area = area.vertical_scroll_offset(top);
        } else if top + row_h > current + view_h {
            area = area.vertical_scroll_offset(top + row_h - view_h);
        }
    }
    st.columns = 1;
    area.show_rows(ui, row_h, total, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for index in range {
            let Some(id) = st.displayed.get(index).cloned() else { continue };
            let Some(item) = core.store.find_shortcut(&id).cloned() else { continue };
            let width = ui.available_width() - 18.0;
            let (outer, response) = ui.allocate_exact_size(vec2(width, row_h), Sense::click_and_drag());
            let outer = Rect::from_min_max(pos2(outer.left() + 14.0, outer.top()), outer.max);
            let selected = st.selected.contains(&id);
            if selected {
                ui.painter().rect_filled(outer.shrink(CARD_MARGIN - 2.0), CornerRadius::same(10), p.surface_selected);
            }
            let card = outer.shrink(CARD_MARGIN);
            let fill = if response.hovered() && !selected { p.surface_hover } else { p.surface };
            ui.painter().rect(card, CornerRadius::same(8), fill, Stroke::new(1.0, p.border), StrokeKind::Inside);

            let inner = card.shrink2(vec2(10.0, 8.0));
            let icon_rect = Rect::from_center_size(pos2(inner.left() + icon / 2.0, inner.center().y), vec2(icon, icon));
            let icon_ref = core.icons.resolve(&item, core.settings.use_browser_icon_cache, core.is_dark());
            widgets::paint_icon(ui, icon_rect, &icon_ref);

            let right_w = (inner.width() * 0.35).min(280.0);
            let text_left = icon_rect.right() + 12.0;
            let text_right = inner.right() - right_w - 12.0;
            paint_text(
                ui,
                pos2(text_left, inner.center().y - 9.0),
                item.display_title(),
                style::bold(style::BODY),
                p.text_primary,
                text_right - text_left,
                false,
            );
            paint_text(
                ui,
                pos2(text_left, inner.center().y + 9.0),
                &item.target,
                style::regular(style::CAPTION),
                p.text_secondary,
                text_right - text_left,
                false,
            );
            let tags = item.tags_text();
            paint_text(
                ui,
                pos2(inner.right(), inner.center().y - 9.0),
                &tags,
                style::regular(style::CAPTION),
                p.text_secondary,
                right_w,
                true,
            );
            paint_text(
                ui,
                pos2(inner.right(), inner.center().y + 9.0),
                &item.folder_path_text,
                style::regular(style::CAPTION),
                p.text_secondary,
                right_w,
                true,
            );

            item_interaction(ui, st, core, &response, index, &id);
        }
    });
}

fn card_view(ui: &mut Ui, st: &mut MainState, core: &mut Core, p: &Palette, list_id: egui::Id) {
    let icon = core.settings.icon_size as f32;
    let cell = vec2(CARD_W + CARD_MARGIN * 2.0, CARD_H + CARD_MARGIN * 2.0);
    let width = ui.available_width() - 22.0;
    let columns = ((width / cell.x).floor() as usize).max(1);
    st.columns = columns;
    let rows = st.displayed.len().div_ceil(columns);
    let mut area = egui::ScrollArea::vertical().id_salt(list_id.with("scroll")).auto_shrink([false, false]);
    if std::mem::take(&mut st.scroll_to_top) {
        area = area.vertical_scroll_offset(0.0);
    }
    if std::mem::take(&mut st.scroll_to_cursor)
        && let Some(c) = st.cursor
    {
        let state = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(list_id.with("scroll")));
        let view_h = ui.available_height();
        let current = state.map(|s| s.clamped_offset().y).unwrap_or(0.0);
        let top = (c / columns) as f32 * cell.y;
        if top < current {
            area = area.vertical_scroll_offset(top);
        } else if top + cell.y > current + view_h {
            area = area.vertical_scroll_offset(top + cell.y - view_h);
        }
    }
    area.show_rows(ui, cell.y, rows, |ui, range| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for row in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(14.0);
                for col in 0..columns {
                    let index = row * columns + col;
                    let Some(id) = st.displayed.get(index).cloned() else { break };
                    let Some(item) = core.store.find_shortcut(&id).cloned() else { continue };
                    let (outer, response) = ui.allocate_exact_size(cell, Sense::click_and_drag());
                    let selected = st.selected.contains(&id);
                    if selected {
                        ui.painter().rect_filled(outer.shrink(2.0), CornerRadius::same(12), p.surface_selected);
                    }
                    let card = outer.shrink(CARD_MARGIN);
                    let fill = if response.hovered() && !selected { p.surface_hover } else { p.surface };
                    ui.painter().rect(card, CornerRadius::same(10), fill, Stroke::new(1.0, p.border), StrokeKind::Inside);

                    let inner = card.shrink2(vec2(12.0, 14.0));
                    let icon_rect = Rect::from_center_size(pos2(inner.center().x, inner.top() + icon / 2.0), vec2(icon, icon));
                    let icon_ref = core.icons.resolve(&item, core.settings.use_browser_icon_cache, core.is_dark());
                    widgets::paint_icon(ui, icon_rect, &icon_ref);

                    // タイトルは 2 行まで、はみ出したら「…」
                    let mut job = egui::text::LayoutJob::simple(
                        item.display_title().to_string(),
                        style::bold(style::BODY),
                        p.text_primary,
                        inner.width(),
                    );
                    job.halign = Align::Center;
                    job.wrap.max_rows = 2;
                    job.wrap.break_anywhere = true;
                    let galley = ui.painter().layout_job(job);
                    let title_top = icon_rect.bottom() + 10.0;
                    ui.painter().galley(pos2(inner.center().x, title_top), galley.clone(), p.text_primary);
                    if !item.tags.is_empty() {
                        // タグは中央にそろえる（はみ出したら「…」）
                        let mut job =
                            egui::text::LayoutJob::simple_singleline(item.tags_text(), style::regular(style::CAPTION), p.text_secondary);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(inner.width());
                        let tags_galley = ui.painter().layout_job(job);
                        let pos = pos2(inner.center().x - tags_galley.size().x / 2.0, title_top + galley.size().y + 6.0);
                        ui.painter().galley(pos, tags_galley, p.text_secondary);
                    }

                    let response = response.on_hover_ui(|ui| {
                        ui.set_max_width(440.0);
                        ui.label(widgets::rich(item.display_title(), style::bold(style::BODY), p.text_primary));
                        ui.label(widgets::rich(&item.target, style::regular(style::CAPTION), p.text_secondary));
                        ui.label(widgets::rich(&item.folder_path_text, style::regular(style::CAPTION), p.text_secondary));
                    });
                    item_interaction(ui, st, core, &response, index, &id);
                }
            });
        }
    });
}

/// 1 行の文字を描く。はみ出したら「…」で切り詰める。right_align なら pos を右端とみなす。
fn paint_text(ui: &Ui, pos: egui::Pos2, text: &str, font: egui::FontId, color: Color32, max_width: f32, right_align: bool) {
    if text.is_empty() || max_width <= 4.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
    let galley = ui.painter().layout_job(job);
    let x = if right_align { pos.x - galley.size().x } else { pos.x };
    ui.painter().galley(pos2(x, pos.y - galley.size().y / 2.0), galley, color);
}

/// 項目のクリック・ダブルクリック・右クリック・ドラッグ。
fn item_interaction(ui: &mut Ui, st: &mut MainState, core: &mut Core, response: &egui::Response, index: usize, id: &str) {
    let modifiers = ui.input(|i| i.modifiers);
    if response.clicked() {
        if modifiers.ctrl {
            if !st.selected.remove(id) {
                st.selected.insert(id.to_string());
            }
            st.anchor = Some(index);
            st.cursor = Some(index);
        } else if modifiers.shift {
            st.select_range(index);
        } else {
            st.select_only(index);
        }
        st.focus_list = true;
    }
    if response.double_clicked() {
        st.select_only(index);
        core.push(Command::Launch { ids: vec![id.to_string()], from_main: true });
    }
    if response.secondary_clicked() && !st.selected.contains(id) {
        // 右クリックした項目が未選択なら、その項目だけを選択してからメニューを出す
        st.select_only(index);
    }
    if response.drag_started() {
        if !st.selected.contains(id) {
            st.select_only(index);
        }
        response.dnd_set_drag_payload(ShortcutDrag(st.selected_ids()));
    }
    response.context_menu(|ui| item_menu(ui, st, core));
}

fn item_menu(ui: &mut Ui, st: &mut MainState, core: &mut Core) {
    ui.set_min_width(220.0);
    let ids = st.selected_ids();
    let first = ids.first().cloned();
    if ui.add(egui::Button::new(loc::t("Str.Menu.Open")).shortcut_text("Enter")).clicked() {
        core.push(Command::Launch { ids: ids.clone(), from_main: true });
        ui.close();
    }
    if ui.add(egui::Button::new(loc::t("Str.Menu.Edit")).shortcut_text("F2")).clicked() {
        if let Some(id) = &first {
            core.push(Command::OpenEdit { existing: Some(id.clone()), folder: None, initial: None, owner: Owner::Main });
        }
        ui.close();
    }
    ui.separator();
    if ui.button(loc::t("Str.Menu.Duplicate")).clicked() {
        if let Some(id) = &first {
            duplicate(core, id);
        }
        ui.close();
    }
    if ui.add(egui::Button::new(loc::t("Str.Menu.CopyTarget")).shortcut_text("Ctrl+C")).clicked() {
        copy_targets(core, &ids, false);
        ui.close();
    }
    if ui.button(loc::t("Str.Menu.CopyTitle")).clicked() {
        copy_targets(core, &ids, true);
        ui.close();
    }
    if ui.button(loc::t("Str.Menu.Reveal")).clicked() {
        if let Some(id) = &first {
            core.push(Command::Reveal { id: id.clone() });
        }
        ui.close();
    }
    ui.separator();
    if ui.button(loc::t("Str.Menu.RefreshIcon")).clicked() {
        core.push(Command::RefreshIcons { ids: ids.clone() });
        ui.close();
    }
    if ui.button(loc::t("Str.Menu.MoveTo")).clicked() {
        core.push(Command::OpenMoveToFolder { ids: ids.clone() });
        ui.close();
    }
    ui.separator();
    if ui.add(egui::Button::new(loc::t("Str.Common.Delete")).shortcut_text("Delete")).clicked() {
        core.push(Command::DeleteShortcuts { ids });
        ui.close();
    }
}

fn duplicate(core: &mut Core, id: &str) {
    let Some(item) = core.store.find_shortcut(id).cloned() else { return };
    let mut copy = item.clone();
    copy.id = crate::model::new_id();
    copy.title = loc::tf("Str.Main.CopySuffix", &[&item.title]);
    copy.usage_count = 0;
    copy.last_used_at = None;
    if let Err(e) = core.store.add_shortcut(copy) {
        crate::log::warn_err("複製に失敗しました。", &e);
    }
}

fn copy_targets(core: &mut Core, ids: &[String], titles: bool) {
    let lines: Vec<String> = ids
        .iter()
        .filter_map(|id| core.store.find_shortcut(id))
        .map(|s| if titles { s.display_title().to_string() } else { s.target.clone() })
        .collect();
    if lines.is_empty() {
        return;
    }
    if crate::platform::write_clipboard_text(&lines.join("\r\n")) {
        core.status(loc::t("Str.Main.Copied"));
    } else {
        crate::log::warn("クリップボードへのコピーに失敗しました。");
    }
}

// ---------------------------------------------------------------- キー操作

fn search_keys(ui: &mut Ui, st: &mut MainState, core: &mut Core) {
    // IME が処理中のキーは egui に届かないので、ここでは変換中の Enter などを奪わない
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) && !st.search.is_empty() {
        st.search.clear();
        st.applied_query.clear();
        st.search_changed_at = None;
    }
    let down = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
    let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    if (down || enter) && !st.displayed.is_empty() {
        // 入力中の検索語をすぐ反映してから、一覧の先頭を選ぶ
        if st.search_changed_at.take().is_some() {
            st.applied_query = st.search.clone();
            st.refresh_list(core);
        }
        if st.displayed.is_empty() {
            return;
        }
        st.select_only(0);
        st.focus_list = true;
        st.scroll_to_cursor = true;
        if enter {
            core.push(Command::Launch { ids: vec![st.displayed[0].clone()], from_main: true });
        }
    }
}

fn list_keys(ui: &mut Ui, st: &mut MainState, core: &mut Core) {
    let count = st.displayed.len();
    if count == 0 {
        return;
    }
    let shift = ui.input(|i| i.modifiers.shift);
    let columns = st.columns.max(1);
    let card = core.settings.view_mode == ViewMode::Card;

    let move_to = |st: &mut MainState, target: usize| {
        let target = target.min(count - 1);
        if shift {
            st.select_range(target);
        } else {
            st.select_only(target);
        }
        st.scroll_to_cursor = true;
    };
    let current = st.cursor.unwrap_or(0);
    let (down, up, left, right, home, end, page_down, page_up) = ui.input_mut(|i| {
        let m = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
        (
            i.consume_key(m, Key::ArrowDown),
            i.consume_key(m, Key::ArrowUp),
            i.consume_key(m, Key::ArrowLeft),
            i.consume_key(m, Key::ArrowRight),
            i.consume_key(m, Key::Home),
            i.consume_key(m, Key::End),
            i.consume_key(m, Key::PageDown),
            i.consume_key(m, Key::PageUp),
        )
    });
    let step = if card { columns } else { 1 };
    if down {
        move_to(st, current + step);
    } else if up {
        move_to(st, current.saturating_sub(step));
    } else if card && right {
        move_to(st, current + 1);
    } else if card && left {
        move_to(st, current.saturating_sub(1));
    } else if home {
        move_to(st, 0);
    } else if end {
        move_to(st, count - 1);
    } else if page_down {
        move_to(st, current + step * 5);
    } else if page_up {
        move_to(st, current.saturating_sub(step * 5));
    }

    let (enter, delete, f2, copy, select_all) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Delete),
            i.consume_key(Modifiers::NONE, Key::F2),
            i.consume_key(Modifiers::COMMAND, Key::C) || i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
            i.consume_key(Modifiers::COMMAND, Key::A),
        )
    });
    let ids = st.selected_ids();
    if enter && !ids.is_empty() {
        core.push(Command::Launch { ids: ids.clone(), from_main: true });
    }
    if delete && !ids.is_empty() {
        core.push(Command::DeleteShortcuts { ids: ids.clone() });
    }
    if f2 && let Some(id) = ids.first() {
        core.push(Command::OpenEdit { existing: Some(id.clone()), folder: None, initial: None, owner: Owner::Main });
    }
    if copy && !ids.is_empty() {
        copy_targets(core, &ids, false);
    }
    if select_all {
        st.selected = st.displayed.iter().cloned().collect();
    }
}

/// ウィンドウ全体のショートカットキー（Ctrl+F / Ctrl+N / Ctrl+V）。
fn handle_window_shortcuts(ui: &mut Ui, st: &mut MainState, core: &mut Core) {
    // 入力欄にフォーカスがあるか（一覧などにフォーカスがあるときは、入力中とはみなさない）
    let typing = ui.ctx().text_edit_focused();
    let (find, new, paste) = ui.input_mut(|i| {
        let find = i.consume_key(Modifiers::COMMAND, Key::F);
        let new = i.consume_key(Modifiers::COMMAND, Key::N);
        // 入力欄の外での Ctrl+V は「クリップボードから登録」（入力欄では通常の貼り付け）
        let paste = !typing && (i.consume_key(Modifiers::COMMAND, Key::V) || i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        (find, new, paste)
    });
    if find {
        st.focus_search = true;
        st.select_all_search = true;
    }
    if new {
        core.push(Command::OpenEdit { existing: None, folder: st.current_folder_id(), initial: None, owner: Owner::Main });
    }
    if paste {
        core.push(Command::RegisterFromClipboard);
    }
}

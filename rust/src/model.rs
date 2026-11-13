//! データの形（C# 版の Models と同じ意味・同じ値）。DB には整数で保存するため、既存の値は変えないこと。

use chrono::{DateTime, Utc};

use crate::loc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TargetType {
    #[default]
    Unknown = 0,
    Web = 1,
    Folder = 2,
    File = 3,
    Application = 4,
}

impl TargetType {
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Web,
            2 => Self::Folder,
            3 => Self::File,
            4 => Self::Application,
            _ => Self::Unknown,
        }
    }

    pub fn display_name(self) -> String {
        loc::t(match self {
            Self::Web => "Str.Type.Web",
            Self::Folder => "Str.Type.Folder",
            Self::File => "Str.Type.File",
            Self::Application => "Str.Type.Application",
            Self::Unknown => "Str.Type.Unknown",
        })
    }

    pub fn is_path(self) -> bool {
        matches!(self, Self::Folder | Self::File | Self::Application)
    }
}

/// フォルダ。parent_id によって階層構造を表す。
#[derive(Clone, Debug)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub sort_order: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 「仕事 > 開発」のような表示用フルパス（ストアが再計算する）
    pub full_path: String,
    /// 子フォルダの ID（表示順、ストアが再計算する）
    pub children: Vec<String>,
}

impl Folder {
    pub fn new(name: &str, parent_id: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: new_id(),
            name: name.to_string(),
            parent_id,
            sort_order: 0,
            created_at: now,
            updated_at: now,
            full_path: String::new(),
            children: Vec::new(),
        }
    }
}

/// 1 件のショートカット。
#[derive(Clone, Debug, Default)]
pub struct Shortcut {
    pub id: String,
    pub title: String,
    /// URL またはローカル / UNC パス
    pub target: String,
    pub target_type: TargetType,
    pub folder_id: Option<String>,
    /// アイコンフォルダ内の相対ファイル名。None の場合は対象種別から自動で決める
    pub icon_path: Option<String>,
    pub note: Option<String>,
    pub sort_order: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// タグ名（表示順）
    pub tags: Vec<String>,
    pub usage_count: i64,
    pub last_used_at: Option<DateTime<Utc>>,

    // ---- 表示用のみ（DB には保存しない） ----
    /// フォルダのフルパス表示（ストアが更新）
    pub folder_path_text: String,
    /// 検索の索引を作り直すかどうかの判定用。内容が変わるたびに増やす
    pub revision: u64,
}

impl Shortcut {
    pub fn new() -> Self {
        let now = Utc::now();
        Self { id: new_id(), created_at: now, updated_at: now, ..Default::default() }
    }

    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() { &self.target } else { &self.title }
    }

    pub fn tags_text(&self) -> String {
        self.tags.join("  ")
    }

    /// 内容が変わったことを検索の索引などに知らせる。
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

/// C# の Guid.NewGuid().ToString("N") と同じ形式（ハイフンなし 32 桁の小文字 16 進）。
pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Card = 0,
    List = 1,
}

/// 配色。Auto は Windows のアプリの配色（ライト / ダーク）に合わせる。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AppTheme {
    Light = 0,
    Dark = 1,
    #[default]
    Auto = 2,
}

/// 表示言語。Auto は Windows の表示言語に合わせる。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AppLanguage {
    #[default]
    Auto = 0,
    Japanese = 1,
    English = 2,
}

/// アプリ設定。DB の settings テーブル（key/value）に保存される。キーは C# 版と同じ。
#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    pub view_mode: ViewMode,
    /// カード/リストのアイコン表示サイズ（px）
    pub icon_size: i32,
    pub theme: AppTheme,
    pub language: AppLanguage,
    /// ランチャー用グローバルホットキー（WPF の ModifierKeys の値。C# 版と共通）
    pub hotkey_modifiers: i32,
    /// ランチャー用グローバルホットキー（WPF の Key の値。C# 版と共通）
    pub hotkey_key: i32,
    pub clipboard_hotkey_modifiers: i32,
    pub clipboard_hotkey_key: i32,
    pub minimize_to_tray: bool,
    pub start_minimized: bool,
    pub run_at_startup: bool,
    pub use_browser_icon_cache: bool,
    pub show_recent_in_launcher: bool,
    pub launcher_max_results: i32,
    pub restore_last_folder: bool,
    pub auto_backup: bool,
    pub last_folder_id: Option<String>,
    pub main_window_width: f64,
    pub main_window_height: f64,
    pub folder_pane_width: f64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::Card,
            icon_size: 32,
            theme: AppTheme::Auto,
            language: AppLanguage::Auto,
            hotkey_modifiers: 2, // Control
            hotkey_key: 18,      // Key.Space
            clipboard_hotkey_modifiers: 0,
            clipboard_hotkey_key: 0,
            minimize_to_tray: true,
            start_minimized: false,
            run_at_startup: false,
            use_browser_icon_cache: true,
            show_recent_in_launcher: true,
            launcher_max_results: 12,
            restore_last_folder: true,
            auto_backup: true,
            last_folder_id: None,
            main_window_width: 1040.0,
            main_window_height: 680.0,
            folder_pane_width: 240.0,
        }
    }
}

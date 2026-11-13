//! 設定画面（C# 版 SettingsWindow と同じ構成、§39, §63, §66）。

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align, CornerRadius, Event, Key, Layout, Stroke, Ui, vec2};
use winit::window::Window;

use super::{card, indented_caption, message, take_escape};
use crate::app::{Command, Core, windows_uses_dark_mode};
use crate::keys;
use crate::loc;
use crate::model::{AppLanguage, AppSettings, AppTheme, ViewMode};
use crate::paths;
use crate::platform::{self, MsgIcon};
use crate::transfer;
use crate::ui::style::{self, Palette};
use crate::ui::widgets::{self, ButtonKind};

const MAX_RESULT_CHOICES: [i32; 5] = [6, 8, 12, 20, 30];
const ICON_SIZES: [(i32, &str); 4] =
    [(24, "Str.Settings.IconSmall"), (32, "Str.Settings.IconNormal"), (48, "Str.Settings.IconLarge"), (64, "Str.Settings.IconXLarge")];
const LABEL_WIDTH: f32 = 150.0;

/// キー入力を待っている呼び出しキー。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    None,
    Launcher,
    Clipboard,
}

pub struct SettingsDialog {
    s: AppSettings,
    capturing: Capture,
    hotkey_status: Option<String>,
    clipboard_status: Option<String>,
    run_at_startup: bool,
    pending_data_directory: Option<PathBuf>,
    reset_data_directory: bool,
    clean_result: String,
    browsers: String,
    /// 閉じるときにプレビュー用のテーマと言語を元に戻す必要があるか
    previewed: bool,
}

impl SettingsDialog {
    pub fn new(core: &Core) -> Self {
        Self {
            s: core.settings.clone(),
            capturing: Capture::None,
            hotkey_status: None,
            clipboard_status: None,
            run_at_startup: platform::startup_enabled(),
            pending_data_directory: None,
            reset_data_directory: false,
            clean_result: String::new(),
            browsers: describe_detected_browsers(),
            previewed: false,
        }
    }

    pub fn cancel(&mut self, core: &mut Core) {
        // プレビュー用に変えていたテーマと言語を元に戻す
        if self.previewed {
            core.push(Command::SettingsCancelled);
            self.previewed = false;
        }
    }

    fn save(&mut self, core: &mut Core, window: &Arc<Window>) {
        let mut s = self.s.clone();
        s.run_at_startup = self.run_at_startup;
        let previous = core.settings.clone();
        core.save_settings(s);
        // テーマ・言語・ホットキー・自動起動の反映はアプリ本体で行う（失敗したらそこで知らせる）
        core.push(Command::SettingsSaved { previous });
        self.previewed = false;

        if self.reset_data_directory || self.pending_data_directory.is_some() {
            let target = if self.reset_data_directory { None } else { self.pending_data_directory.as_deref() };
            match paths::save_data_directory_override(target) {
                Ok(()) => message(window, &loc::t("Str.Settings.LocationChanged"), &loc::t("Str.Settings.LocationTitle"), MsgIcon::Info),
                Err(e) => {
                    crate::log::error_err("保存場所の変更に失敗しました。", &e);
                    message(window, &loc::tf("Str.Settings.LocationFailed", &[&e]), &loc::t("Str.Common.Error"), MsgIcon::Error);
                }
            }
        }
    }

    // ------------------------------------------------------------- ホットキー

    fn toggle_capturing(&mut self, target: Capture) {
        if self.capturing == target {
            self.stop_capturing();
            return;
        }
        self.stop_capturing();
        self.capturing = target;
        let hint = Some(loc::t("Str.Settings.CaptureHint"));
        match target {
            Capture::Launcher => self.hotkey_status = hint,
            Capture::Clipboard => self.clipboard_status = hint,
            Capture::None => {}
        }
    }

    fn stop_capturing(&mut self) {
        self.capturing = Capture::None;
        self.hotkey_status = None;
        self.clipboard_status = None;
    }

    /// キー入力を受け付ける（受け付けている間は、ほかの操作にキーを渡さない）。
    fn capture_keys(&mut self, ui: &mut Ui) {
        if self.capturing == Capture::None {
            return;
        }
        let events: Vec<(Key, egui::Modifiers)> = ui.input_mut(|i| {
            let mut keys = Vec::new();
            i.events.retain(|e| match e {
                Event::Key { key, pressed: true, modifiers, .. } => {
                    keys.push((*key, *modifiers));
                    false
                }
                Event::Key { .. } | Event::Text(_) => false,
                _ => true,
            });
            keys
        });
        for (key, m) in events {
            if key == Key::Escape {
                self.stop_capturing();
                return;
            }
            let vk = keys::vk_from_egui(key);
            let wpf = keys::wpf_from_vk(vk);
            if wpf == 0 {
                continue;
            }
            let mut modifiers = 0;
            if m.ctrl {
                modifiers |= keys::MOD_CONTROL;
            }
            if m.shift {
                modifiers |= keys::MOD_SHIFT;
            }
            if m.alt {
                modifiers |= keys::MOD_ALT;
            }
            if platform::windows_key_down() {
                modifiers |= keys::MOD_WINDOWS;
            }
            let status = if modifiers == 0 {
                Some(loc::t("Str.Settings.NeedModifier"))
            } else if self.capturing == Capture::Launcher {
                // 2 つの呼び出しキーに同じ組み合わせは使えない
                if modifiers == self.s.clipboard_hotkey_modifiers && wpf == self.s.clipboard_hotkey_key {
                    Some(loc::t("Str.Settings.HotKeySameAsClipboard"))
                } else {
                    self.s.hotkey_modifiers = modifiers;
                    self.s.hotkey_key = wpf;
                    self.stop_capturing();
                    return;
                }
            } else if modifiers == self.s.hotkey_modifiers && wpf == self.s.hotkey_key {
                Some(loc::t("Str.Settings.HotKeySameAsLauncher"))
            } else {
                self.s.clipboard_hotkey_modifiers = modifiers;
                self.s.clipboard_hotkey_key = wpf;
                self.stop_capturing();
                return;
            };
            match self.capturing {
                Capture::Launcher => self.hotkey_status = status,
                Capture::Clipboard => self.clipboard_status = status,
                Capture::None => {}
            }
        }
    }

    // ---------------------------------------------------------------- 画面

    pub fn ui(&mut self, ui: &mut Ui, core: &mut Core, window: &Arc<Window>) -> bool {
        let p = core.palette;
        let capturing_before = self.capturing != Capture::None;
        self.capture_keys(ui);

        let mut close = false;
        egui::Panel::bottom("settings-buttons")
            .frame(egui::Frame::new().fill(p.header_background).inner_margin(egui::Margin::symmetric(20, 12)))
            .show_separator_line(false)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                ui.painter().hline(rect.x_range(), rect.top() - 12.0, Stroke::new(1.0, p.border));
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button_ex(ui, &p, ButtonKind::Primary, &loc::t("Str.Common.Save"), true, Some(100.0)).clicked() {
                            self.save(core, window);
                            close = true;
                        }
                        if widgets::button_ex(ui, &p, ButtonKind::Secondary, &loc::t("Str.Common.Cancel"), true, Some(100.0)).clicked() {
                            self.cancel(core);
                            close = true;
                        }
                    });
                });
            });

        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 16, top: 18, bottom: 4 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    self.display_section(ui, core, &p);
                    self.launcher_section(ui, core, &p);
                    self.startup_section(ui, &p);
                    self.icons_section(ui, core, &p);
                    self.data_section(ui, core, &p, window);
                    self.about_section(ui, core, &p);
                });
            });
        });

        if close {
            return true;
        }
        // キーの受け付け中の Esc は受け付けの中止だけにする
        if !capturing_before && take_escape(ui) {
            self.cancel(core);
            return true;
        }
        false
    }

    fn display_section(&mut self, ui: &mut Ui, core: &mut Core, p: &Palette) {
        section_title(ui, p, &loc::t("Str.Settings.Display"));
        card(ui, p, |ui| {
            // 言語。「日本語」「English」は常にその言語自身の表記で出し、誤って切り替えても元に戻せるようにする
            let languages = [
                (AppLanguage::Auto, loc::t("Str.Settings.LanguageAuto")),
                (AppLanguage::Japanese, "日本語".to_string()),
                (AppLanguage::English, "English".to_string()),
            ];
            let before = self.s.language;
            row(ui, &loc::t("Str.Settings.Language"), |ui, w| {
                combo(ui, "settings-language", &languages, &mut self.s.language, w);
            });
            if self.s.language != before {
                self.previewed = true;
                core.push(Command::PreviewLanguage(self.s.language));
            }
            note_row(ui, p, &loc::describe_auto_detection());

            ui.add_space(12.0);
            let themes = [
                (AppTheme::Auto, loc::t("Str.Settings.ThemeAuto")),
                (AppTheme::Light, loc::t("Str.Settings.ThemeLight")),
                (AppTheme::Dark, loc::t("Str.Settings.ThemeDark")),
            ];
            let before = self.s.theme;
            row(ui, &loc::t("Str.Settings.Theme"), |ui, w| {
                combo(ui, "settings-theme", &themes, &mut self.s.theme, w);
            });
            if self.s.theme != before {
                // テーマは選んだ瞬間に反映して、見た目を確認できるようにする
                self.previewed = true;
                core.push(Command::PreviewTheme(self.s.theme));
            }
            let current = loc::t(if windows_uses_dark_mode() { "Str.Settings.ThemeDark" } else { "Str.Settings.ThemeLight" });
            note_row(ui, p, &loc::tf("Str.Settings.ThemeAutoNote", &[&current]));

            ui.add_space(12.0);
            let views = [(ViewMode::Card, loc::t("Str.Main.CardViewTooltip")), (ViewMode::List, loc::t("Str.Main.ListViewTooltip"))];
            row(ui, &loc::t("Str.Settings.ViewMode"), |ui, w| {
                combo(ui, "settings-view", &views, &mut self.s.view_mode, w);
            });

            ui.add_space(12.0);
            let sizes: Vec<(i32, String)> = ICON_SIZES.iter().map(|(v, k)| (*v, loc::t(k))).collect();
            if !sizes.iter().any(|(v, _)| *v == self.s.icon_size) {
                self.s.icon_size = 32;
            }
            row(ui, &loc::t("Str.Settings.IconSize"), |ui, w| {
                combo(ui, "settings-iconsize", &sizes, &mut self.s.icon_size, w);
            });
        });
    }

    fn launcher_section(&mut self, ui: &mut Ui, core: &mut Core, p: &Palette) {
        section_title(ui, p, &loc::t("Str.Settings.Launcher"));
        card(ui, p, |ui| {
            // ---- ランチャーの呼び出しキー
            let capturing = self.capturing == Capture::Launcher;
            let text =
                if capturing { loc::t("Str.Settings.PressKeys") } else { keys::describe(self.s.hotkey_modifiers, self.s.hotkey_key) };
            let mut toggle = false;
            row(ui, &loc::t("Str.Settings.HotKey"), |ui, _w| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = loc::t(if capturing { "Str.Common.Cancel" } else { "Str.Settings.Change" });
                    toggle = widgets::button_ex(ui, p, ButtonKind::Secondary, &label, true, Some(80.0)).clicked();
                    ui.add_space(8.0);
                    let w = ui.available_width();
                    hotkey_box(ui, p, &text, w, capturing);
                });
            });
            if toggle {
                self.toggle_capturing(Capture::Launcher);
            }
            // 登録に失敗したキーをそのまま表示しているときだけ、使用中である旨を出す
            let unchanged = core.settings.hotkey_modifiers == self.s.hotkey_modifiers && core.settings.hotkey_key == self.s.hotkey_key;
            let status = self.hotkey_status.clone().unwrap_or_else(|| {
                loc::t(if core.hotkey_failed && unchanged { "Str.Settings.HotKeyInUse" } else { "Str.Settings.HotKeyConflictHint" })
            });
            ui.add_space(8.0);
            indented_caption(ui, p, LABEL_WIDTH, &status);

            // ---- クリップボードから登録（他のアプリのキーと競合しやすいので、既定は未設定）
            ui.add_space(14.0);
            let capturing = self.capturing == Capture::Clipboard;
            let text = if capturing {
                loc::t("Str.Settings.PressKeys")
            } else {
                keys::describe(self.s.clipboard_hotkey_modifiers, self.s.clipboard_hotkey_key)
            };
            let mut toggle = false;
            let mut clear = false;
            row(ui, &loc::t("Str.Settings.ClipboardHotKey"), |ui, _w| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    clear = widgets::button(ui, p, ButtonKind::Ghost, &loc::t("Str.Settings.ClearHotKey")).clicked();
                    ui.add_space(4.0);
                    let label = loc::t(if capturing { "Str.Common.Cancel" } else { "Str.Settings.Change" });
                    toggle = widgets::button_ex(ui, p, ButtonKind::Secondary, &label, true, Some(80.0)).clicked();
                    ui.add_space(8.0);
                    let w = ui.available_width();
                    hotkey_box(ui, p, &text, w, capturing);
                });
            });
            if toggle {
                self.toggle_capturing(Capture::Clipboard);
            }
            if clear {
                self.s.clipboard_hotkey_modifiers = 0;
                self.s.clipboard_hotkey_key = 0;
                self.stop_capturing();
            }
            let unchanged = core.settings.clipboard_hotkey_modifiers == self.s.clipboard_hotkey_modifiers
                && core.settings.clipboard_hotkey_key == self.s.clipboard_hotkey_key;
            let status = self.clipboard_status.clone().unwrap_or_else(|| {
                loc::t(if core.clipboard_hotkey_failed && unchanged {
                    "Str.Settings.HotKeyInUse"
                } else {
                    "Str.Settings.ClipboardHotKeyNote"
                })
            });
            ui.add_space(8.0);
            indented_caption(ui, p, LABEL_WIDTH, &status);

            ui.add_space(14.0);
            ui.checkbox(&mut self.s.show_recent_in_launcher, loc::t("Str.Settings.ShowRecent"));

            ui.add_space(14.0);
            let choices: Vec<(i32, String)> = MAX_RESULT_CHOICES.iter().map(|c| (*c, loc::tf("Str.Common.CountItems", &[c]))).collect();
            if !choices.iter().any(|(v, _)| *v == self.s.launcher_max_results) {
                self.s.launcher_max_results = 12;
            }
            row(ui, &loc::t("Str.Settings.MaxResults"), |ui, w| {
                combo(ui, "settings-maxresults", &choices, &mut self.s.launcher_max_results, w);
            });
        });
    }

    fn startup_section(&mut self, ui: &mut Ui, p: &Palette) {
        section_title(ui, p, &loc::t("Str.Settings.Startup"));
        card(ui, p, |ui| {
            ui.checkbox(&mut self.s.minimize_to_tray, loc::t("Str.Settings.MinimizeToTray"));
            ui.add_space(4.0);
            indented_caption(ui, p, 25.0, &loc::t("Str.Settings.MinimizeToTrayNote"));

            ui.add_space(14.0);
            ui.checkbox(&mut self.s.start_minimized, loc::t("Str.Settings.StartMinimized"));

            ui.add_space(14.0);
            ui.checkbox(&mut self.run_at_startup, loc::t("Str.Settings.RunAtStartup"));
            ui.add_space(4.0);
            indented_caption(ui, p, 25.0, &loc::t("Str.Settings.RunAtStartupNote"));

            ui.add_space(14.0);
            ui.checkbox(&mut self.s.restore_last_folder, loc::t("Str.Settings.RestoreFolder"));
        });
    }

    fn icons_section(&mut self, ui: &mut Ui, core: &mut Core, p: &Palette) {
        section_title(ui, p, &loc::t("Str.Settings.Icons"));
        card(ui, p, |ui| {
            ui.checkbox(&mut self.s.use_browser_icon_cache, loc::t("Str.Settings.BrowserIconCache"));
            ui.add_space(4.0);
            indented_caption(ui, p, 25.0, &loc::t("Str.Settings.BrowserIconCacheNote"));
            ui.add_space(6.0);
            indented_caption(ui, p, 25.0, &self.browsers);

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, p, ButtonKind::Secondary, &loc::t("Str.Settings.CleanIcons")).clicked() {
                    let removed = core.icons.clean_up_unused_icons(&core.store);
                    self.clean_result =
                        if removed == 0 { loc::t("Str.Settings.CleanNone") } else { loc::tf("Str.Settings.CleanDone", &[&removed]) };
                }
                ui.add_space(2.0);
                ui.label(widgets::rich(&self.clean_result, style::regular(style::CAPTION), p.text_secondary));
            });
        });
    }

    fn data_section(&mut self, ui: &mut Ui, core: &mut Core, p: &Palette, window: &Arc<Window>) {
        section_title(ui, p, &loc::t("Str.Settings.Data"));
        card(ui, p, |ui| {
            ui.label(loc::t("Str.Settings.DataLocation"));
            ui.add_space(6.0);
            let current = paths::data_directory().display().to_string();
            let text = if self.reset_data_directory {
                loc::tf("Str.Settings.NextLaunchDefault", &[&current, &paths::default_data_directory().display()])
            } else if let Some(pending) = &self.pending_data_directory {
                loc::tf("Str.Settings.NextLaunch", &[&current, &pending.display()])
            } else {
                current
            };
            egui::Frame::new()
                .fill(p.input_background)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(widgets::rich(&text, style::regular(12.0), p.text_primary)).wrap());
                });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, p, ButtonKind::Secondary, &loc::t("Str.Settings.OpenFolder")).clicked() {
                    crate::launch::open_path(paths::data_directory().to_string_lossy());
                }
                if widgets::button(ui, p, ButtonKind::Secondary, &loc::t("Str.Settings.ChangeLocation")).clicked()
                    && let Some(folder) = rfd::FileDialog::new()
                        .set_title(loc::t("Str.Settings.ChooseDataFolder"))
                        .set_directory(paths::data_directory())
                        .set_parent(window.as_ref())
                        .pick_folder()
                {
                    self.pending_data_directory = Some(folder);
                    self.reset_data_directory = false;
                }
                if widgets::button(ui, p, ButtonKind::Ghost, &loc::t("Str.Common.ResetDefault")).clicked() {
                    self.pending_data_directory = None;
                    self.reset_data_directory = true;
                }
            });
            ui.add_space(8.0);
            widgets::caption(
                ui,
                p,
                &loc::t(if paths::using_fallback_location() { "Str.Settings.DataNoteFallback" } else { "Str.Settings.DataNoteDefault" }),
            );

            ui.add_space(16.0);
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, p.border);
            ui.add_space(14.0);

            ui.checkbox(&mut self.s.auto_backup, loc::t("Str.Settings.AutoBackup"));
            ui.add_space(4.0);
            let last = match transfer::last_auto_backup_time() {
                Some(time) => loc::tf("Str.Settings.AutoBackupLast", &[&format_general_time(&time)]),
                None => loc::t("Str.Settings.AutoBackupNone"),
            };
            let note = format!("{}\n{}", loc::tf("Str.Settings.AutoBackupNote", &[&transfer::AUTO_KEEP_COUNT]), last);
            indented_caption(ui, p, 25.0, &note);

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, p, ButtonKind::Secondary, &loc::t("Str.Settings.OpenBackup")).clicked() {
                    let dir = paths::backup_directory();
                    let _ = std::fs::create_dir_all(&dir);
                    crate::launch::open_path(dir.to_string_lossy());
                }
                if widgets::button(ui, p, ButtonKind::Secondary, &loc::t("Str.Settings.OpenLog")).clicked() {
                    let log = paths::log_file();
                    if log.is_file() {
                        crate::launch::open_path(log.to_string_lossy());
                    } else {
                        message(window, &loc::t("Str.Settings.NoLog"), &loc::t("Str.Settings.LogTitle"), MsgIcon::Info);
                    }
                }
            });
            let _ = core;
        });
    }

    fn about_section(&mut self, ui: &mut Ui, core: &mut Core, p: &Palette) {
        section_title(ui, p, &loc::t("Str.Settings.About"));
        card(ui, p, |ui| {
            let version = format!("{} (Rust)", env!("CARGO_PKG_VERSION"));
            ui.label(loc::tf("Str.Settings.Version", &[&loc::t("Str.App.Name"), &version]));
            ui.add_space(6.0);
            let stats = loc::tf("Str.Settings.Stats", &[&core.store.shortcut_count(), &core.store.folder_count(), &core.store.tag_count()]);
            widgets::caption(ui, p, &stats);
        });
    }
}

/// 見出し（SectionHeaderText）とカードの間隔。
fn section_title(ui: &mut Ui, p: &Palette, text: &str) {
    widgets::section_header(ui, p, text);
    ui.add_space(6.0);
}

/// 「ラベル（幅 150）| 中身」の 1 行。中身には残りの幅を渡す。
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui, f32)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 30.0), egui::Sense::hover());
        let mut job =
            egui::text::LayoutJob::simple(label.to_string(), style::regular(style::BODY), ui.visuals().text_color(), LABEL_WIDTH - 12.0);
        job.wrap.max_rows = 2;
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, ui.visuals().text_color());
        let w = ui.available_width();
        add(ui, w);
    });
}

/// 行の下の補足（ラベルの列の分だけ右へずらす）。
fn note_row(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(6.0);
    indented_caption(ui, p, LABEL_WIDTH + 2.0, text);
}

fn combo<T: PartialEq + Copy>(ui: &mut Ui, id: &str, choices: &[(T, String)], value: &mut T, width: f32) {
    let current = choices.iter().find(|(v, _)| v == value).map(|(_, l)| l.clone()).unwrap_or_default();
    egui::ComboBox::from_id_salt(id).selected_text(current).width(width).show_ui(ui, |ui| {
        for (v, label) in choices {
            ui.selectable_value(value, *v, label);
        }
    });
}

/// 呼び出しキーの表示欄。
fn hotkey_box(ui: &mut Ui, p: &Palette, text: &str, width: f32, capturing: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        p.input_background,
        Stroke::new(1.0, if capturing { p.accent } else { p.border_strong }),
        egui::StrokeKind::Inside,
    );
    let galley = ui.painter().layout_no_wrap(text.to_string(), style::bold(style::BODY), p.text_primary);
    ui.painter().galley(egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, p.text_primary);
}

/// この PC で参照できるブラウザ（設定の効果を分かりやすくするため）。
fn describe_detected_browsers() -> String {
    let browsers = crate::favicon_cache::detect_available_browsers();
    if browsers.is_empty() {
        loc::t("Str.Settings.NoBrowsers")
    } else {
        loc::tf("Str.Settings.BrowsersFound", &[&browsers.join(&loc::t("Str.Common.ListSeparator"))])
    }
}

/// C# の ToString("g")（日付と短い時刻）に近い表示。
fn format_general_time(time: &chrono::DateTime<chrono::Local>) -> String {
    if loc::current() == AppLanguage::English {
        time.format("%-m/%-d/%Y %-I:%M %p").to_string()
    } else {
        time.format("%Y/%m/%d %-H:%M").to_string()
    }
}

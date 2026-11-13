//! ショートカットのアイコン解決（C# 版 IconService の移植）。
//!
//!   Web        : ブラウザが保存しているアイコンを icons/ にコピーして使う（アプリからサイトへの通信はしない）
//!   フォルダ   : Windows のフォルダアイコン
//!   ファイル   : 関連付けられたアプリのアイコン
//!   カスタム   : ユーザーが指定した画像を icons/ にコピー
//!
//! DB にはバイナリではなく icons/ 配下の相対ファイル名だけを保存する（C# 版と同じファイル名の規則）。
//! 画面の表示を待たせないよう、ブラウザのファイルの読み取り・共有フォルダの存在確認と
//! アイコンの取得は裏のスレッドで行い、終わったら結果を画面のスレッドへ送る。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::ColorImage;
use sha1::Digest;

use crate::favicon_cache::BrowserFaviconCache;
use crate::model::{Shortcut, TargetType};
use crate::paths;
use crate::shell_icons;
use crate::store::Store;
use crate::target;

const FAVICON_EXTENSIONS: &[&str] = &[".png", ".ico", ".jpg", ".gif", ".bmp"];
/// 同じパスの存在を確認し直す間隔。
const EXISTENCE_RECHECK: Duration = Duration::from_secs(5);
/// 一覧に出すアイコンの最大の大きさ（これより大きい画像は縮小して持つ）。
const MAX_ICON_PIXELS: u32 = 96;

/// 表示するアイコン。Image の key はウィンドウごとのテクスチャのキャッシュに使う。
#[derive(Clone)]
pub enum IconRef {
    Image { key: String, image: Arc<ColorImage> },
    Default(TargetType),
}

/// 裏の処理から画面のスレッドへ送る結果。
enum IconEvent {
    Existence { key: String, exists: bool },
    ShellLoaded,
    FaviconFound { origin: String, file_name: String, force_item: Option<String> },
    FaviconNotFound { origin: String, force_item: Option<String> },
}

struct FaviconJob {
    origin: String,
    page: url::Url,
    file_base: String,
    dark: bool,
    force_item: Option<String>,
}

pub struct IconService {
    /// ファイル名 -> 読み込み済み画像
    memory: HashMap<String, Arc<ColorImage>>,
    /// ファイル名 -> 差し替えた回数（同じ名前で中身が変わったとき、表示を作り直させるため）
    versions: HashMap<String, u64>,
    /// icons/ にあるファイル名（大文字にそろえたもの）。None なら次に必要になったときに読む
    icon_files: Option<HashSet<String>>,
    /// ブラウザに見つからなかったサイト（アプリから開くか「アイコンを再取得」するまで探し直さない）
    failed: HashSet<String>,
    in_flight: HashSet<String>,
    /// "d|パス" / "f|パス" -> (存在するか, 確認した時刻)
    existence: HashMap<String, (bool, Instant)>,
    existence_pending: HashSet<String>,
    shell_pending: HashSet<String>,

    events_rx: Receiver<IconEvent>,
    existence_tx: Sender<(String, String, bool)>,
    shell_tx: Sender<(String, bool)>,
    favicon_tx: Sender<FaviconJob>,
    browser_cache: Arc<BrowserFaviconCache>,

    /// アイコンが変わるたびに増える番号（画面はこれを見て描き直す）
    pub generation: u64,
    /// 「再取得」の結果（項目の ID, 見つかったアイコンのファイル名）。画面が受け取って状態表示に使う
    pub last_force_results: Vec<(String, Option<String>)>,
    /// 保存済みのアイコンを反映する項目（項目の ID, ファイル名）
    pending_apply: Vec<(String, String)>,
}

impl IconService {
    /// wake は裏の処理が終わったときに呼ばれる（画面のスレッドを起こす）。
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        let browser_cache = Arc::new(BrowserFaviconCache::default());

        // 存在確認（同時に 2 件まで）
        let (existence_tx, existence_rx) = mpsc::channel::<(String, String, bool)>();
        let existence_rx = Arc::new(Mutex::new(existence_rx));
        for _ in 0..2 {
            let rx = existence_rx.clone();
            let tx = events_tx.clone();
            let wake = wake.clone();
            let _ = std::thread::Builder::new().name("ExistenceCheck".into()).spawn(move || {
                loop {
                    let job = { rx.lock().ok().and_then(|r| r.recv().ok()) };
                    let Some((key, path, folder)) = job else { break };
                    let exists = if folder { Path::new(&path).is_dir() } else { Path::new(&path).is_file() };
                    if tx.send(IconEvent::Existence { key, exists }).is_err() {
                        break;
                    }
                    wake();
                }
            });
        }

        // 共有フォルダ上のアイコンの取得（シェルは STA で呼ぶ必要があるので専用スレッド）
        let (shell_tx, shell_rx) = mpsc::channel::<(String, bool)>();
        {
            let tx = events_tx.clone();
            let wake = wake.clone();
            let _ = std::thread::Builder::new().name("ShellIconLoader".into()).spawn(move || {
                let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
                while let Ok((path, folder)) = shell_rx.recv() {
                    if folder {
                        let _ = shell_icons::folder_icon(&path, true);
                    } else {
                        let _ = shell_icons::file_icon(&path, true);
                    }
                    if tx.send(IconEvent::ShellLoaded).is_err() {
                        break;
                    }
                    wake();
                }
            });
        }

        // ブラウザのアイコンキャッシュの読み取り（1 件ずつ。一時コピーを使い回すため）
        let (favicon_tx, favicon_rx) = mpsc::channel::<FaviconJob>();
        {
            let tx = events_tx.clone();
            let wake = wake.clone();
            let cache = browser_cache.clone();
            let _ = std::thread::Builder::new().name("BrowserFavicon".into()).spawn(move || {
                loop {
                    match favicon_rx.recv_timeout(Duration::from_secs(5)) {
                        Ok(job) => {
                            let found = try_browser_cache(&cache, &job.page, &job.file_base, job.dark);
                            let event = match found {
                                Some(file_name) => IconEvent::FaviconFound { origin: job.origin, file_name, force_item: job.force_item },
                                None => IconEvent::FaviconNotFound { origin: job.origin, force_item: job.force_item },
                            };
                            if tx.send(event).is_err() {
                                break;
                            }
                            wake();
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => cache.expire_snapshots(),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                cache.dispose();
            });
        }

        Self {
            memory: HashMap::new(),
            versions: HashMap::new(),
            icon_files: None,
            failed: HashSet::new(),
            in_flight: HashSet::new(),
            existence: HashMap::new(),
            existence_pending: HashSet::new(),
            shell_pending: HashSet::new(),
            events_rx,
            existence_tx,
            shell_tx,
            favicon_tx,
            browser_cache,
            generation: 0,
            last_force_results: Vec::new(),
            pending_apply: Vec::new(),
        }
    }

    pub fn full_path(name: &str) -> PathBuf {
        paths::icon_directory().join(name)
    }

    // ------------------------------------------------------------ 表示用の解決

    /// 一覧に表示するアイコン。Web で favicon 未取得なら、裏で探し始める。
    pub fn resolve(&mut self, item: &Shortcut, use_browser_cache: bool, dark: bool) -> IconRef {
        if let Some(name) = item.icon_path.as_deref().filter(|n| !n.is_empty())
            && let Some(icon) = self.load_icon_file(name)
        {
            return icon;
        }

        let path = target::expand(&item.target);
        match item.target_type {
            TargetType::Folder => self.shell_icon_for_list(&path, true).unwrap_or(IconRef::Default(TargetType::Folder)),
            TargetType::File => self.shell_icon_for_list(&path, false).unwrap_or(IconRef::Default(TargetType::File)),
            TargetType::Application => self.shell_icon_for_list(&path, false).unwrap_or(IconRef::Default(TargetType::Application)),
            TargetType::Web => {
                if item.icon_path.is_none() {
                    self.request_favicon(item, false, use_browser_cache, dark);
                }
                IconRef::Default(TargetType::Web)
            }
            TargetType::Unknown => IconRef::Default(TargetType::Unknown),
        }
    }

    /// 登録画面のプレビュー用（存在確認は呼び出し側で済ませた結果を使う）。
    pub fn resolve_preview(&mut self, kind: TargetType, target_text: &str, icon_path: Option<&str>, exists: bool) -> IconRef {
        if let Some(name) = icon_path.filter(|n| !n.is_empty())
            && let Some(icon) = self.load_icon_file(name)
        {
            return icon;
        }
        let path = target::expand(target_text);
        let folder = kind == TargetType::Folder;
        if !kind.is_path() {
            return IconRef::Default(kind);
        }
        if exists && !shell_icons::is_loaded(&path, folder) && target::is_network_path(&path) {
            // 共有フォルダの実物のアイコンは裏で取得し、取得できたら描き直す
            let placeholder = if folder { shell_icons::folder_icon(&path, false) } else { shell_icons::file_icon(&path, false) };
            if !shell_icons::is_loaded(&path, folder) {
                self.load_shell_icon_in_background(&path, folder);
            }
            return placeholder.map(shell_ref).unwrap_or(IconRef::Default(kind));
        }
        let icon = if folder { shell_icons::folder_icon(&path, exists) } else { shell_icons::file_icon(&path, exists) };
        icon.map(shell_ref).unwrap_or(IconRef::Default(kind))
    }

    /// フォルダ / ファイルのシェルアイコン。共有フォルダ上のもので実物を読まないと分からない
    /// アイコンをまだ取得していなければ、いったん標準のアイコンを返し、実物は裏で取得してから差し替える。
    fn shell_icon_for_list(&mut self, path: &str, folder: bool) -> Option<IconRef> {
        let exists = self.known_exists(path, folder);
        if exists && !shell_icons::is_loaded(path, folder) && target::is_network_path(path) {
            let placeholder = if folder { shell_icons::folder_icon(path, false) } else { shell_icons::file_icon(path, false) };
            // 一般的なファイルのアイコンは拡張子で決まるので、標準のアイコンを取得した時点で用が済む
            if !shell_icons::is_loaded(path, folder) {
                self.load_shell_icon_in_background(path, folder);
            }
            return placeholder.map(shell_ref);
        }
        let icon = if folder { shell_icons::folder_icon(path, exists) } else { shell_icons::file_icon(path, exists) };
        icon.map(shell_ref)
    }

    /// パスが存在するか。前回の確認結果があればそれを返し、古くなっていれば裏で確認し直す。
    fn known_exists(&mut self, path: &str, folder: bool) -> bool {
        if path.trim().is_empty() {
            return false;
        }
        let key = format!("{}{}", if folder { "d|" } else { "f|" }, path);
        if let Some(&(exists, checked)) = self.existence.get(&key) {
            if checked.elapsed() >= EXISTENCE_RECHECK {
                self.queue_existence_check(key, path, folder);
            }
            return exists;
        }

        // 初めて表示するパス。ローカルはその場で確かめ、ネットワーク上のパスは
        // つながらないと長く待たされるので、いったん「無い」ものとして表示し、裏で確かめる
        if target::is_network_path(path) {
            self.existence.insert(key.clone(), (false, Instant::now()));
            self.queue_existence_check(key, path, folder);
            return false;
        }
        let exists = if folder { Path::new(path).is_dir() } else { Path::new(path).is_file() };
        self.existence.insert(key, (exists, Instant::now()));
        exists
    }

    fn queue_existence_check(&mut self, key: String, path: &str, folder: bool) {
        if !self.existence_pending.insert(key.clone()) {
            return; // 確認中
        }
        let _ = self.existence_tx.send((key, path.to_string(), folder));
    }

    fn load_shell_icon_in_background(&mut self, path: &str, folder: bool) {
        let key = format!("{}{}", if folder { "d|" } else { "f|" }, path);
        if self.shell_pending.insert(key) {
            let _ = self.shell_tx.send((path.to_string(), folder));
        }
    }

    // ---------------------------------------------------------- 裏の処理の結果

    /// 裏の処理の結果を受け取り、反映する。アイコンが変わったら true（描き直しが必要）。
    pub fn poll(&mut self, store: &mut Store) -> bool {
        let mut changed = false;
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                IconEvent::Existence { key, exists } => {
                    self.existence_pending.remove(&key);
                    let before = self.existence.insert(key, (exists, Instant::now())).map(|e| e.0);
                    if before != Some(exists) {
                        changed = true;
                    }
                }
                IconEvent::ShellLoaded => {
                    self.shell_pending.clear();
                    changed = true;
                }
                IconEvent::FaviconFound { origin, file_name, force_item } => {
                    self.in_flight.remove(&origin);
                    self.failed.remove(&origin);
                    self.memory.remove(&file_name);
                    *self.versions.entry(file_name.clone()).or_default() += 1;
                    self.remember_icon_file(&file_name);
                    self.apply_to_origin(store, &origin, &file_name);
                    if let Some(id) = force_item {
                        if let Err(e) = store.update_icon_path(&id, Some(&file_name)) {
                            crate::log::warn_err("アイコンパスの保存に失敗しました。", &e);
                        }
                        self.last_force_results.push((id, Some(file_name.clone())));
                    }
                    changed = true;
                }
                IconEvent::FaviconNotFound { origin, force_item } => {
                    self.in_flight.remove(&origin);
                    self.failed.insert(origin);
                    if let Some(id) = force_item {
                        self.last_force_results.push((id, None));
                    }
                }
            }
        }
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
        changed
    }

    /// 同じオリジンの、カスタムアイコンを持たないショートカットすべてに反映する。
    fn apply_to_origin(&mut self, store: &mut Store, origin: &str, file_name: &str) {
        let targets: Vec<String> = store
            .shortcuts()
            .iter()
            .filter(|s| s.target_type == TargetType::Web)
            .filter(|s| s.icon_path.as_deref().is_none_or(|p| p.starts_with("fav_")))
            .filter(|s| target::try_get_origin(&s.target).is_some_and(|o| o.eq_ignore_ascii_case(origin)))
            .map(|s| s.id.clone())
            .collect();
        for id in targets {
            if let Err(e) = store.update_icon_path(&id, Some(file_name)) {
                crate::log::warn_err("アイコンパスの保存に失敗しました。", &e);
            }
        }
    }

    // ---------------------------------------------------------------- favicon

    /// Web ショートカットのアイコンを、ブラウザが保存しているアイコンから取り出す（裏で行う）。
    /// force なら保存済みのアイコンがあっても探し直し、結果を last_force_results に残す。
    pub fn request_favicon(&mut self, item: &Shortcut, force: bool, use_browser_cache: bool, dark: bool) -> bool {
        if item.target_type != TargetType::Web {
            return false;
        }
        let Some(page) = target::try_get_uri(&item.target) else { return false };
        if page.scheme() != "http" && page.scheme() != "https" {
            return false;
        }
        let Some(origin) = target::try_get_origin(&item.target) else { return false };
        let file_base = favicon_file_base(&origin);

        if !force {
            // すでに保存済みのアイコンがあればそれを使う（同じサイトの別のショートカットが取得済みなど）
            if let Some(existing) = self.find_cached_favicon(&file_base) {
                self.pending_apply.push((item.id.clone(), existing));
                return true;
            }
            if self.failed.contains(&origin) {
                return false;
            }
        }
        if !use_browser_cache {
            return false;
        }
        if !self.in_flight.insert(origin.clone()) && !force {
            return false; // 同じサイトを探している最中
        }
        let _ = self.favicon_tx.send(FaviconJob { origin, page, file_base, dark, force_item: force.then(|| item.id.clone()) });
        true
    }

    /// 起動したショートカットのアイコンを取り直せるよう、「見つからなかった」記録を消す
    /// （ブラウザで開いた後なら取れる可能性があるため。C# 版と同じく 5 秒後・20 秒後に試す）。
    pub fn forget_failure(&mut self, item: &Shortcut) {
        if let Some(origin) = target::try_get_origin(&item.target) {
            self.failed.remove(&origin);
        }
    }

    fn find_cached_favicon(&mut self, file_base: &str) -> Option<String> {
        FAVICON_EXTENSIONS.iter().map(|ext| format!("{file_base}{ext}")).find(|name| self.icon_file_exists(name))
    }

    // ------------------------------------------------------------ icons/ の一覧

    fn icon_file_exists(&mut self, name: &str) -> bool {
        let files = self.icon_files.get_or_insert_with(load_icon_file_names);
        if !files.contains(&name.to_uppercase()) {
            return false;
        }
        if Self::full_path(name).is_file() {
            return true;
        }
        files.remove(&name.to_uppercase());
        false
    }

    fn remember_icon_file(&mut self, name: &str) {
        if let Some(files) = self.icon_files.as_mut() {
            files.insert(name.to_uppercase());
        }
    }

    fn forget_icon_file(&mut self, name: &str) {
        if let Some(files) = self.icon_files.as_mut() {
            files.remove(&name.to_uppercase());
        }
    }

    /// icons/ の画像を読み込む（読み込んだものはメモリに持っておく）。
    fn load_icon_file(&mut self, name: &str) -> Option<IconRef> {
        let version = self.versions.get(name).copied().unwrap_or(0);
        let key = format!("file:{}@{}", name.to_lowercase(), version);
        if let Some(image) = self.memory.get(name) {
            return Some(IconRef::Image { key, image: image.clone() });
        }
        let path = Self::full_path(name);
        if !path.is_file() {
            return None;
        }
        let image = Arc::new(load_image_file(&path)?);
        self.memory.insert(name.to_string(), image.clone());
        Some(IconRef::Image { key, image })
    }

    /// 画面の表示（アイコン一覧）をすべて読み直させる（インポート後や「再取得」時）。
    pub fn clear_memory_cache(&mut self) {
        self.memory.clear();
        self.failed.clear();
        self.existence.clear();
        self.icon_files = None;
        shell_icons::clear_cache();
        self.generation = self.generation.wrapping_add(1);
    }

    // ---------------------------------------------------------- カスタムアイコン

    /// ユーザーが選んだ画像を icons/ にコピーして、相対ファイル名を返す。
    pub fn import_custom_icon(&mut self, source: &Path) -> Option<String> {
        let bytes = fs::read(source).ok()?;
        let ext = detect_image_extension(&bytes)
            .map(str::to_string)
            .or_else(|| source.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())))
            .unwrap_or_else(|| ".png".into());
        let name = format!("custom_{}{}", crate::model::new_id(), ext);
        let full = Self::full_path(&name);
        let _ = fs::create_dir_all(paths::icon_directory());
        if fs::write(&full, &bytes).is_err() {
            return None;
        }
        if load_image_file(&full).is_none() {
            let _ = fs::remove_file(&full);
            return None;
        }
        self.remember_icon_file(&name);
        Some(name)
    }

    /// カスタムアイコンを削除する（favicon は他のショートカットと共有なので消さない）。
    pub fn delete_custom_icon(&mut self, name: Option<&str>) {
        let Some(name) = name.filter(|n| n.starts_with("custom_")) else { return };
        self.memory.remove(name);
        let full = Self::full_path(name);
        if full.is_file()
            && let Err(e) = fs::remove_file(&full)
        {
            crate::log::warn_err(&format!("アイコンの削除に失敗: {name}"), &e);
        }
        self.forget_icon_file(name);
    }

    /// どのショートカットからも参照されていないアイコンファイルを削除する。
    pub fn clean_up_unused_icons(&mut self, store: &Store) -> usize {
        let used: HashSet<String> = store.shortcuts().iter().filter_map(|s| s.icon_path.as_deref()).map(str::to_uppercase).collect();
        let mut removed = 0;
        if let Ok(entries) = fs::read_dir(paths::icon_directory()) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if used.contains(&name.to_uppercase()) {
                    continue;
                }
                if fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        self.memory.clear();
        self.icon_files = None;
        removed
    }

    /// 保存済みのアイコンを、表示している項目へ反映する（request_favicon が見つけたもの）。
    pub fn apply_pending(&mut self, store: &mut Store) -> bool {
        if self.pending_apply.is_empty() {
            return false;
        }
        for (id, name) in std::mem::take(&mut self.pending_apply) {
            if let Err(e) = store.update_icon_path(&id, Some(&name)) {
                crate::log::warn_err("アイコンパスの保存に失敗しました。", &e);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        true
    }
}

impl Drop for IconService {
    fn drop(&mut self) {
        self.browser_cache.dispose();
    }
}

fn shell_ref(image: shell_icons::Icon) -> IconRef {
    IconRef::Image { key: format!("shell:{:p}", Arc::as_ptr(&image)), image }
}

fn load_icon_file_names() -> HashSet<String> {
    let mut names = HashSet::new();
    match fs::read_dir(paths::icon_directory()) {
        Ok(entries) => {
            for entry in entries.flatten() {
                names.insert(entry.file_name().to_string_lossy().to_uppercase());
            }
        }
        Err(e) => crate::log::warn_err("アイコンフォルダの一覧を読めませんでした。", &e),
    }
    names
}

/// "fav_" + オリジンのハッシュ（C# 版と同じ。同じサイトのアイコンを C# 版と共有できる）。
pub fn favicon_file_base(origin: &str) -> String {
    let hash = sha1::Sha1::digest(origin.to_lowercase().as_bytes());
    let hex: String = hash.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("fav_{hex}")
}

/// ブラウザのアイコンキャッシュから取り出して icons/ に保存する（裏のスレッドで呼ぶ）。
fn try_browser_cache(cache: &BrowserFaviconCache, page: &url::Url, file_base: &str, dark: bool) -> Option<String> {
    let found = cache.try_find(page)?;
    // ブラウザはダークモード用の白いアイコンを保存していることがある。
    // 現在のテーマの背景に埋もれて「アイコンが無い」ように見えるものは採用しない
    if is_invisible_on_theme(&found.data, dark) {
        crate::log::info(&format!("背景に埋もれるアイコンのため採用しませんでした: {}", page.host_str().unwrap_or("")));
        return None;
    }
    let saved = save_icon_bytes(&found.data, file_base);
    if saved.is_some() {
        crate::log::info(&format!("{} のアイコンキャッシュから取得しました: {}", found.browser_name, page.host_str().unwrap_or("")));
    }
    saved
}

/// 画像バイト列を icons/ に保存し、相対ファイル名を返す。
/// 一時ファイル経由で置き換えるので、途中で失敗しても壊れたファイルが残らない。
fn save_icon_bytes(bytes: &[u8], file_base: &str) -> Option<String> {
    let ext = detect_image_extension(bytes)?;
    let name = format!("{file_base}{ext}");
    let dir = paths::icon_directory();
    let _ = fs::create_dir_all(&dir);
    let full = dir.join(&name);
    let temp = dir.join(format!("{name}.tmp"));
    fs::write(&temp, bytes).ok()?;
    if load_image_file(&temp).is_none() {
        let _ = fs::remove_file(&temp);
        return None;
    }
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let n = entry.file_name().to_string_lossy().into_owned();
            if n.starts_with(&format!("{file_base}.")) && !n.ends_with(".tmp") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    fs::rename(&temp, &full).ok()?;
    Some(name)
}

/// マジックナンバーから画像形式を判定する（拡張子は当てにしない）。
pub fn detect_image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && bytes[..4] == [0x89, 0x50, 0x4E, 0x47] {
        return Some(".png");
    }
    if bytes.len() >= 4 && bytes[0] == 0 && bytes[1] == 0 && (bytes[2] == 1 || bytes[2] == 2) && bytes[3] == 0 {
        return Some(".ico");
    }
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return Some(".jpg");
    }
    if bytes.len() >= 6 && &bytes[..3] == b"GIF" {
        return Some(".gif");
    }
    if bytes.len() >= 2 && &bytes[..2] == b"BM" {
        return Some(".bmp");
    }
    None
}

/// 画像ファイルを読み込む（ico / png / jpg / gif / bmp）。大きすぎる画像は縮小する。
pub fn load_image_file(path: &Path) -> Option<ColorImage> {
    let bytes = fs::read(path).ok()?;
    decode_image(&bytes).or_else(|| {
        crate::log::warn(&format!("画像を読み込めませんでした: {}", path.display()));
        None
    })
}

pub fn decode_image(bytes: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > MAX_ICON_PIXELS || img.height() > MAX_ICON_PIXELS {
        img.resize(MAX_ICON_PIXELS, MAX_ICON_PIXELS, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// 現在のテーマの背景に埋もれて見えないアイコンかどうか（C# 版と同じ判定）。
fn is_invisible_on_theme(data: &[u8], dark: bool) -> bool {
    let Ok(img) = image::load_from_memory(data) else { return false };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return false;
    }
    let mut visible = 0usize;
    let mut matches_background = 0usize;
    for px in rgba.pixels() {
        let [r, g, b, a] = px.0;
        if a < 40 {
            continue; // ほぼ透明な画素は無視
        }
        visible += 1;
        let luminance = (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64) / 255.0;
        if if dark { luminance < 0.12 } else { luminance > 0.90 } {
            matches_background += 1;
        }
    }
    if (visible as f64) < (w * h) as f64 * 0.02 {
        return true;
    }
    matches_background as f64 >= visible as f64 * 0.97
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favicon_file_names_match_csharp() {
        // C# 版: "fav_" + SHA1(小文字のオリジン) の先頭 16 桁（小文字）
        let base = favicon_file_base("https://example.com");
        assert!(base.starts_with("fav_"));
        assert_eq!(base.len(), 4 + 16);
        assert_eq!(base, favicon_file_base("HTTPS://EXAMPLE.COM"));
    }

    #[test]
    fn detects_image_formats() {
        assert_eq!(detect_image_extension(&[0x89, 0x50, 0x4E, 0x47, 0, 0, 0, 0]), Some(".png"));
        assert_eq!(detect_image_extension(&[0, 0, 1, 0]), Some(".ico"));
        assert_eq!(detect_image_extension(b"<svg></svg>"), None);
    }

    #[test]
    fn white_icons_are_invisible_on_light_theme() {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 255, 255, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        assert!(is_invisible_on_theme(&out, false));
        assert!(!is_invisible_on_theme(&out, true));
    }
}

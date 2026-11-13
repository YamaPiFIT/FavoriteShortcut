//! Windows シェルからファイル / フォルダのアイコンを取得する（C# 版 ShellIconProvider と同じ方法）。
//!
//! 高 DPI でも粗くならないよう、まず 48px の Extra Large アイコン一覧から取得を試み、
//! 取れない環境では SHGetFileInfo の 32px アイコンにフォールバックする。
//! 同じ拡張子のアイコンは使い回すのでファイル数が増えても負荷は上がらない。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::ColorImage;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetObjectW,
    HBITMAP,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES};
use windows::Win32::UI::Controls::IImageList;
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, SHGetImageList,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use windows::core::PCWSTR;

use crate::util::wide;

const SHIL_LARGE: i32 = 0;
const SHIL_EXTRALARGE: i32 = 2;
const ILD_TRANSPARENT: u32 = 1;
const GENERIC_FOLDER_KEY: &str = "dir:*";

pub type Icon = Arc<ColorImage>;

static CACHE: Mutex<Option<HashMap<String, Option<Icon>>>> = Mutex::new(None);

fn cached(key: &str) -> Option<Option<Icon>> {
    CACHE.lock().ok()?.as_ref()?.get(key).cloned()
}

fn store(key: &str, icon: Option<Icon>) {
    if let Ok(mut guard) = CACHE.lock() {
        guard.get_or_insert_with(HashMap::new).insert(key.to_string(), icon);
    }
}

pub fn clear_cache() {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = None;
    }
}

/// 実在するフォルダは固有アイコン（デスクトップ等）を持つことがあるのでパスをキーにする。
fn folder_key(path: &str, exists: bool) -> String {
    if exists && !path.trim().is_empty() { format!("dir:{}", path.to_lowercase()) } else { GENERIC_FOLDER_KEY.to_string() }
}

/// EXE / LNK / ICO などは実体ごとにアイコンが違うのでパスをキーにし、それ以外は拡張子をキーにする。
fn file_key(path: &str, exists: bool) -> String {
    let ext = crate::target::extension(path).to_lowercase();
    let per_file = exists && matches!(ext.as_str(), ".exe" | ".lnk" | ".ico" | ".msi" | ".url");
    if per_file { format!("file:{}", path.to_lowercase()) } else { format!("ext:{}", if ext.is_empty() { "<none>" } else { ext.as_str() }) }
}

/// フォルダの標準アイコン（存在確認は呼び出し側で済ませたもの）。
pub fn folder_icon(path: &str, exists: bool) -> Option<Icon> {
    let key = folder_key(path, exists);
    if let Some(hit) = cached(&key) {
        return hit;
    }
    let icon = if key == GENERIC_FOLDER_KEY {
        load("C:\\", FILE_ATTRIBUTE_DIRECTORY, true)
    } else {
        load(path, FILE_FLAGS_AND_ATTRIBUTES(0), false)
    };
    store(&key, icon.clone());
    icon
}

/// ファイルのアイコン（存在確認は呼び出し側で済ませたもの）。
pub fn file_icon(path: &str, exists: bool) -> Option<Icon> {
    if path.trim().is_empty() {
        return None;
    }
    let key = file_key(path, exists);
    if let Some(hit) = cached(&key) {
        return hit;
    }
    let icon = if key.starts_with("file:") || exists {
        load(path, FILE_FLAGS_AND_ATTRIBUTES(0), false)
    } else {
        let ext = crate::target::extension(path);
        load(&format!("dummy{ext}"), FILE_ATTRIBUTE_NORMAL, true)
    };
    store(&key, icon.clone());
    icon
}

/// 実在するフォルダ / ファイルとしてのアイコンを取得済みか。
/// まだなら、取得するときに実物（共有フォルダならサーバー）を読みに行くことになる。
pub fn is_loaded(path: &str, folder: bool) -> bool {
    if path.trim().is_empty() {
        return true;
    }
    let key = if folder { folder_key(path, true) } else { file_key(path, true) };
    CACHE.lock().ok().and_then(|g| g.as_ref().map(|m| m.contains_key(&key))).unwrap_or(false)
}

fn load(path: &str, attributes: FILE_FLAGS_AND_ATTRIBUTES, use_file_attributes: bool) -> Option<Icon> {
    let wpath = wide(path);
    unsafe {
        let mut info = SHFILEINFOW::default();
        let mut flags = SHGFI_SYSICONINDEX;
        if use_file_attributes {
            flags |= SHGFI_USEFILEATTRIBUTES;
        }
        let r = SHGetFileInfoW(PCWSTR(wpath.as_ptr()), attributes, Some(&mut info), std::mem::size_of::<SHFILEINFOW>() as u32, flags);
        if r != 0 {
            for size in [SHIL_EXTRALARGE, SHIL_LARGE] {
                if let Some(icon) = from_image_list(info.iIcon, size) {
                    return Some(icon);
                }
            }
        }

        // Extra Large が使えない環境向けのフォールバック
        let mut info2 = SHFILEINFOW::default();
        let mut flags2 = SHGFI_ICON | SHGFI_LARGEICON;
        if use_file_attributes {
            flags2 |= SHGFI_USEFILEATTRIBUTES;
        }
        if SHGetFileInfoW(PCWSTR(wpath.as_ptr()), attributes, Some(&mut info2), std::mem::size_of::<SHFILEINFOW>() as u32, flags2) == 0 {
            return None;
        }
        let icon = icon_to_image(info2.hIcon);
        if !info2.hIcon.is_invalid() {
            let _ = DestroyIcon(info2.hIcon);
        }
        icon
    }
}

unsafe fn from_image_list(index: i32, size: i32) -> Option<Icon> {
    unsafe {
        let list: IImageList = SHGetImageList(size).ok()?;
        let hicon = list.GetIcon(index, ILD_TRANSPARENT).ok()?;
        if hicon.is_invalid() {
            return None;
        }
        let icon = icon_to_image(hicon);
        let _ = DestroyIcon(hicon);
        icon
    }
}

/// HICON を RGBA の画像へ変換する。アルファを持たない古い形式はマスクから透明部分を作る。
unsafe fn icon_to_image(hicon: HICON) -> Option<Icon> {
    unsafe {
        if hicon.is_invalid() {
            return None;
        }
        let mut info = ICONINFO::default();
        GetIconInfo(hicon, &mut info).ok()?;
        let result = (|| {
            let color = info.hbmColor;
            if color.is_invalid() {
                return None;
            }
            let (w, h, mut pixels) = read_bitmap(color)?;
            let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
            if !has_alpha
                && !info.hbmMask.is_invalid()
                && let Some((mw, mh, mask)) = read_bitmap(info.hbmMask)
                && mw == w
                && mh >= h
            {
                for (i, px) in pixels.chunks_exact_mut(4).enumerate() {
                    // マスクが黒（0）の画素が見える部分
                    px[3] = if mask[i * 4] == 0 { 255 } else { 0 };
                }
            }
            // BGRA → RGBA
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            Some(Arc::new(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels)))
        })();
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(info.hbmColor.into());
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(info.hbmMask.into());
        }
        result
    }
}

/// ビットマップを 32bpp の上から下への並び（BGRA）で読み出す。
unsafe fn read_bitmap(bitmap: HBITMAP) -> Option<(i32, i32, Vec<u8>)> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(bitmap.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 || w > 1024 || h > 1024 {
            return None;
        }
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 上から下へ
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr() as *mut _), &mut header, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }
        Some((w, h, pixels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_icons_do_not_touch_the_file() {
        // 拡張子で決まるアイコンは、共有フォルダ上のファイルを読まずに取得できる
        let share = "\\\\fsc-test-server\\share";
        let start = std::time::Instant::now();
        let _ = file_icon(&format!("{share}\\見積.fsctest"), false);
        assert!(start.elapsed().as_secs() < 5);
        assert!(is_loaded(&format!("{share}\\別の見積.fsctest"), false), "同じ拡張子は取得済み");
        assert!(!is_loaded(&format!("{share}\\tool.exe"), false), "EXE は実物を読む");
        assert!(!is_loaded(&format!("{share}\\別フォルダ"), true), "フォルダは実物を読む");
    }

    #[test]
    fn local_folder_icon_can_be_read() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        // アイコンを出せない環境（サーバーの検証環境など）では取得できないこともある
        if let Some(icon) = folder_icon(&windir, true) {
            assert!(icon.size[0] >= 16 && icon.size[1] >= 16);
        }
    }
}

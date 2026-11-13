//! 管理画面へのドラッグ＆ドロップの受け付け（Windows の OLE）。
//!
//! winit が扱えるのはファイルのドロップだけで、ドロップした位置も分からない。C# 版と同じく
//!   - エクスプローラーのファイル / フォルダをフォルダツリーの特定のフォルダへドロップして登録する
//!   - ブラウザのアドレスバーなどから URL（文字列）をドロップして登録画面を開く
//! ことができるよう、ドロップを自前で受け付ける。

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, POINTL, S_OK};
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop,
    ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_core::{Ref, implement};

/// ドラッグ＆ドロップの知らせ。位置は画面上の座標（物理ピクセル）。
#[derive(Clone, Debug)]
pub enum DropEvent {
    /// 受け付けられるもの（ファイルか文字列）がウィンドウの上を動いている。files はファイルか
    Over {
        x: i32,
        y: i32,
        files: bool,
    },
    /// ウィンドウの外へ出た / 取りやめた
    Leave,
    Drop {
        x: i32,
        y: i32,
        paths: Vec<String>,
        text: Option<String>,
    },
}

#[implement(IDropTarget)]
struct DropTarget {
    handler: Box<dyn Fn(DropEvent)>,
    /// いまドラッグしているものを受け付けられるか（DragEnter で決める）
    accept: Cell<bool>,
    /// いまドラッグしているものがファイルか
    files: Cell<bool>,
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        let files = data.ok().is_ok_and(|d| has_format(d, CF_HDROP.0));
        let accept = files || data.ok().is_ok_and(|d| has_format(d, CF_UNICODETEXT.0));
        self.accept.set(accept);
        self.files.set(files);
        set_effect(effect, accept);
        if accept {
            (self.handler)(DropEvent::Over { x: pt.x, y: pt.y, files });
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows_core::Result<()> {
        let accept = self.accept.get();
        set_effect(effect, accept);
        if accept {
            (self.handler)(DropEvent::Over { x: pt.x, y: pt.y, files: self.files.get() });
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows_core::Result<()> {
        self.accept.set(false);
        (self.handler)(DropEvent::Leave);
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows_core::Result<()> {
        self.accept.set(false);
        let Ok(data) = data.ok() else {
            set_effect(effect, false);
            (self.handler)(DropEvent::Leave);
            return Ok(());
        };
        let paths = read_paths(data);
        let text = if paths.is_empty() { read_text(data) } else { None };
        let accepted = !paths.is_empty() || text.is_some();
        set_effect(effect, accepted);
        if accepted {
            (self.handler)(DropEvent::Drop { x: pt.x, y: pt.y, paths, text });
        } else {
            (self.handler)(DropEvent::Leave);
        }
        Ok(())
    }
}

fn set_effect(effect: *mut DROPEFFECT, accept: bool) {
    if !effect.is_null() {
        unsafe {
            *effect = if accept { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
    }
}

fn format(cf: u16) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

fn has_format(data: &IDataObject, cf: u16) -> bool {
    let f = format(cf);
    // 形式が無いときは S_FALSE（成功扱いの値）が返るので、S_OK かどうかで判断する
    (unsafe { data.QueryGetData(&f) }) == S_OK
}

/// エクスプローラーからドロップされたファイル / フォルダのパス。
fn read_paths(data: &IDataObject) -> Vec<String> {
    let f = format(CF_HDROP.0);
    let Ok(mut medium) = (unsafe { data.GetData(&f) }) else { return Vec::new() };
    let mut paths = Vec::new();
    unsafe {
        let hglobal = medium.u.hGlobal;
        let ptr = GlobalLock(hglobal);
        if !ptr.is_null() {
            let hdrop = HDROP(ptr);
            let count = DragQueryFileW(hdrop, u32::MAX, None);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let copied = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
                let path = String::from_utf16_lossy(&buf[..copied.min(len)]);
                if !path.is_empty() {
                    paths.push(path);
                }
            }
            let _ = GlobalUnlock(hglobal);
        }
        ReleaseStgMedium(&mut medium);
    }
    paths
}

/// ドロップされた文字列（ブラウザのアドレスバーからドラッグした URL など）。
fn read_text(data: &IDataObject) -> Option<String> {
    let f = format(CF_UNICODETEXT.0);
    let mut medium = unsafe { data.GetData(&f) }.ok()?;
    let mut text = None;
    unsafe {
        let hglobal = medium.u.hGlobal;
        let ptr = GlobalLock(hglobal) as *const u16;
        if !ptr.is_null() {
            // 終端の 0 まで（念のためメモリの大きさを超えて読まない）
            let max = GlobalSize(hglobal) / 2;
            let mut len = 0;
            while len < max && *ptr.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            let s = s.trim().to_string();
            if !s.is_empty() {
                text = Some(s);
            }
            let _ = GlobalUnlock(hglobal);
        }
        ReleaseStgMedium(&mut medium);
    }
    text
}

/// ウィンドウでドロップを受け付ける（winit の受け付けは止めておくこと）。成功したら true。
pub fn register(hwnd: HWND, handler: impl Fn(DropEvent) + 'static) -> bool {
    unsafe {
        // ドラッグ＆ドロップには OLE の初期化が必要（初期化済みなら何もしない）
        let _ = OleInitialize(None);
        let target: IDropTarget = DropTarget { handler: Box::new(handler), accept: Cell::new(false), files: Cell::new(false) }.into();
        match RegisterDragDrop(hwnd, &target) {
            Ok(()) => true,
            Err(e) => {
                crate::log::warn_err("ドラッグ＆ドロップの受け付けを開始できませんでした。", &e);
                false
            }
        }
    }
}

/// ウィンドウを閉じる前に受け付けをやめる。
pub fn revoke(hwnd: HWND) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::mem::ManuallyDrop;
    use std::rc::Rc;

    use windows::Win32::Foundation::{HGLOBAL, POINT};
    use windows::Win32::System::Com::{STGMEDIUM, STGMEDIUM_0};
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc};
    use windows::Win32::UI::Shell::{DROPFILES, SHCreateDataObject};

    use super::*;

    /// HGLOBAL にバイト列を入れる。
    fn global(bytes: &[u8]) -> HGLOBAL {
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).unwrap();
            let p = GlobalLock(h) as *mut u8;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            let _ = GlobalUnlock(h);
            h
        }
    }

    fn data_object(cf: u16, bytes: &[u8]) -> IDataObject {
        unsafe {
            let _ = OleInitialize(None);
            let data: IDataObject = SHCreateDataObject(None, None, None::<&IDataObject>).unwrap();
            let medium = STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: global(bytes) },
                pUnkForRelease: ManuallyDrop::new(None),
            };
            data.SetData(&format(cf), &medium, true).unwrap();
            data
        }
    }

    fn utf16_bytes(s: &str) -> Vec<u8> {
        s.encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect()
    }

    /// エクスプローラーのドラッグと同じ形（DROPFILES + 0 区切りのパス）。
    fn hdrop_bytes(paths: &[&str]) -> Vec<u8> {
        let header =
            DROPFILES { pFiles: std::mem::size_of::<DROPFILES>() as u32, pt: POINT::default(), fNC: false.into(), fWide: true.into() };
        let mut bytes =
            unsafe { std::slice::from_raw_parts(&header as *const DROPFILES as *const u8, std::mem::size_of::<DROPFILES>()).to_vec() };
        for p in paths {
            bytes.extend(utf16_bytes(p));
        }
        bytes.extend([0, 0]);
        bytes
    }

    fn target() -> (IDropTarget, Rc<RefCell<Vec<DropEvent>>>) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let target: IDropTarget =
            DropTarget { handler: Box::new(move |e| sink.borrow_mut().push(e)), accept: Cell::new(false), files: Cell::new(false) }.into();
        (target, events)
    }

    #[test]
    fn url_text_is_accepted_and_delivered() {
        let data = data_object(CF_UNICODETEXT.0, &utf16_bytes("  https://www.example.com/  "));
        let (target, events) = target();
        let mut effect = DROPEFFECT_NONE;
        unsafe {
            target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 10, y: 20 }, &mut effect).unwrap();
            assert_eq!(effect, DROPEFFECT_COPY);
            target.Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 30, y: 40 }, &mut effect).unwrap();
        }
        let events = events.borrow();
        assert!(matches!(events[0], DropEvent::Over { x: 10, y: 20, files: false }));
        match &events[1] {
            DropEvent::Drop { x, y, paths, text } => {
                assert_eq!((*x, *y), (30, 40));
                assert!(paths.is_empty());
                assert_eq!(text.as_deref(), Some("https://www.example.com/"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn explorer_files_are_accepted_and_delivered() {
        let data = data_object(CF_HDROP.0, &hdrop_bytes(&[r"C:\Windows", r"C:\テスト\メモ.txt"]));
        let (target, events) = target();
        let mut effect = DROPEFFECT_NONE;
        unsafe {
            target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 1, y: 2 }, &mut effect).unwrap();
            assert_eq!(effect, DROPEFFECT_COPY);
            target.DragOver(MODIFIERKEYS_FLAGS(0), POINTL { x: 3, y: 4 }, &mut effect).unwrap();
            target.Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 5, y: 6 }, &mut effect).unwrap();
        }
        let events = events.borrow();
        assert!(matches!(events[0], DropEvent::Over { files: true, .. }));
        assert!(matches!(events[1], DropEvent::Over { x: 3, y: 4, files: true }));
        match &events[2] {
            DropEvent::Drop { paths, text, .. } => {
                assert_eq!(paths, &vec![r"C:\Windows".to_string(), r"C:\テスト\メモ.txt".to_string()]);
                assert!(text.is_none());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn other_formats_are_refused() {
        // 画像など、受け付けない形式だけのドラッグ（CF_DIB = 8）
        let data = data_object(8, &[0u8; 64]);
        let (target, events) = target();
        let mut effect = DROPEFFECT_COPY;
        unsafe {
            target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 0, y: 0 }, &mut effect).unwrap();
        }
        assert_eq!(effect, DROPEFFECT_NONE);
        assert!(events.borrow().is_empty());
    }
}

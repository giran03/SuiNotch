//! Drag and drop for the shelf: files and text dropped on the notch are kept (by path, never
//! copied), and items can be dragged back out into any app.

use std::mem::ManuallyDrop;

use windows::core::{implement, Ref, Result};
use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::System::Com::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::*;

use crate::app;
use crate::notch::{self, Page};
use crate::store::ShelfItem;
use crate::sys;

const CF_UNICODETEXT: u16 = 13;
const CF_HDROP: u16 = 15;

fn format(cf: u16) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

#[implement(IDropTarget)]
pub struct DropTarget {
    hwnd: HWND,
    helper: Option<IDropTargetHelper>,
}

fn accepts(data: &IDataObject) -> bool {
    unsafe { data.QueryGetData(&format(CF_HDROP)).is_ok() || data.QueryGetData(&format(CF_UNICODETEXT)).is_ok() }
}

fn read(data: &IDataObject) -> (Vec<std::path::PathBuf>, Option<String>) {
    unsafe {
        if let Ok(mut medium) = data.GetData(&format(CF_HDROP)) {
            let paths = sys::paths_from_hdrop(HDROP(medium.u.hGlobal.0));
            ReleaseStgMedium(&mut medium);
            if !paths.is_empty() {
                return (paths, None);
            }
        }
        if let Ok(mut medium) = data.GetData(&format(CF_UNICODETEXT)) {
            let ptr = GlobalLock(medium.u.hGlobal) as *const u16;
            let text = if ptr.is_null() {
                None
            } else {
                let mut len = 0;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                let _ = GlobalUnlock(medium.u.hGlobal);
                Some(s)
            };
            ReleaseStgMedium(&mut medium);
            return (Vec::new(), text);
        }
    }
    (Vec::new(), None)
}

fn pt(p: &POINTL) -> POINT {
    POINT { x: p.x, y: p.y }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, p: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let ok = data.as_ref().is_some_and(accepts);
        let own = app::with(|app| {
            if ok && app.cfg.shelf_open_on_drag {
                app.notch.drag_over = true;
                notch::request_frame(app);
            }
            app.notch.own_drag
        })
        .unwrap_or(false);
        let e = if ok && !own { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        unsafe {
            *effect = e;
            if let (Some(h), Some(d)) = (&self.helper, data.as_ref()) {
                let _ = h.DragEnter(self.hwnd, d, &pt(p), e);
            }
        }
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, p: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let own = app::with(|app| app.notch.own_drag).unwrap_or(false);
        let e = if own { DROPEFFECT_NONE } else { DROPEFFECT_COPY };
        unsafe {
            *effect = e;
            if let Some(h) = &self.helper {
                let _ = h.DragOver(&pt(p), e);
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        app::with(|app| {
            app.notch.drag_over = false;
            notch::request_frame(app);
        });
        if let Some(h) = &self.helper {
            unsafe {
                let _ = h.DragLeave();
            }
        }
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, p: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let (paths, text) = data.as_ref().map(read).unwrap_or_default();
        let own = app::with(|app| app.notch.own_drag).unwrap_or(false);
        app::with(|app| {
            app.notch.drag_over = false;
            if own {
                notch::request_frame(app);
                return;
            }
            let added = if !paths.is_empty() {
                app.shelf.add_paths(&paths) > 0
            } else {
                text.as_deref().is_some_and(|t| app.shelf.add_text(t))
            };
            if added {
                app.save_shelf();
                let max = crate::views::shelf_overflow(app);
                app.notch.shelf_scroll.set(max);
            }
            notch::set_page(app, Page::Shelf);
            notch::expand(app);
            notch::request_frame(app);
        });
        unsafe {
            *effect = if own { DROPEFFECT_NONE } else { DROPEFFECT_COPY };
            if let (Some(h), Some(d)) = (&self.helper, data.as_ref()) {
                let _ = h.Drop(d, &pt(p), *effect);
            }
        }
        Ok(())
    }
}

pub fn register(hwnd: HWND) {
    unsafe {
        let helper: Option<IDropTargetHelper> = CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok();
        let target: IDropTarget = DropTarget { hwnd, helper }.into();
        if let Err(e) = RegisterDragDrop(hwnd, &target) {
            crate::log(&format!("drop target unavailable: {e}"));
        }
    }
}

fn files_object(paths: &[std::path::PathBuf]) -> Option<IDataObject> {
    unsafe {
        let pidls: Vec<_> = paths
            .iter()
            .map(|p| ILCreateFromPathW(&windows::core::HSTRING::from(p.as_os_str())))
            .filter(|p| !p.is_null())
            .collect();
        if pidls.is_empty() {
            return None;
        }
        let consts: Vec<*const _> = pidls.iter().map(|p| *p as *const _).collect();
        let array = SHCreateShellItemArrayFromIDLists(&consts);
        for p in &pidls {
            ILFree(Some(*p));
        }
        array.ok()?.BindToHandler(None, &BHID_DataObject).ok()
    }
}

fn text_object(text: &str) -> Option<IDataObject> {
    unsafe {
        let data: IDataObject = SHCreateDataObject(None, None, None).ok()?;
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
        let ptr = GlobalLock(mem) as *mut u16;
        if ptr.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(mem);
        let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: mem }, pUnkForRelease: ManuallyDrop::new(None) };
        data.SetData(&format(CF_UNICODETEXT), &medium, true).ok()?;
        Some(data)
    }
}

/// Runs the system drag loop for the given shelf items. Must be called with the app unborrowed.
/// Drags a clipboard-history item out: its text, or its image as a file.
pub fn drag_clip(i: usize) {
    use crate::clips::Clip;
    let Some((hwnd, clip)) = app::with(|app| {
        app.notch.own_drag = true;
        (app.notch.hwnd, app.clips.items.get(i).cloned())
    }) else {
        return;
    };
    let data = match &clip {
        Some(Clip::Text(t)) => text_object(t),
        Some(Clip::Image { path, .. }) => files_object(std::slice::from_ref(path)),
        None => None,
    };
    if let Some(d) = data {
        unsafe {
            let _ = SHDoDragDrop(Some(hwnd), &d, None::<&IDropSource>, DROPEFFECT_COPY);
        }
    }
    app::with(|app| {
        app.notch.own_drag = false;
        app.notch.drag_over = false;
        notch::request_frame(app);
    });
}

pub fn drag_out(indices: Vec<usize>) {
    let Some((hwnd, items)) = app::with(|app| {
        app.notch.own_drag = true;
        let items: Vec<ShelfItem> = indices.iter().filter_map(|&i| app.shelf.items.get(i).cloned()).collect();
        (app.notch.hwnd, items)
    }) else {
        return;
    };
    let paths: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            ShelfItem::File { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    let data = if !paths.is_empty() {
        files_object(&paths)
    } else {
        items.iter().find_map(|i| match i {
            ShelfItem::Text { text } => text_object(text),
            _ => None,
        })
    };
    let effect = match data {
        Some(d) => unsafe {
            SHDoDragDrop(Some(hwnd), &d, None::<&IDropSource>, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK)
                .unwrap_or(DROPEFFECT_NONE)
        },
        None => DROPEFFECT_NONE,
    };
    app::with(|app| {
        app.notch.own_drag = false;
        app.notch.drag_over = false;
        let mut changed = app.shelf.prune_missing();
        if effect != DROPEFFECT_NONE && app.cfg.shelf_remove_after_drag {
            app.shelf.remove_indices(&indices);
            changed = true;
        }
        if changed {
            app.notch.shelf_sel.clear();
            app.notch.bitmaps.clear();
            app.save_shelf();
        }
        notch::request_frame(app);
    });
}

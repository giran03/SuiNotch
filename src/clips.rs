//! Clipboard history for the Shelf: the last texts and images copied anywhere, newest first.
//! Kept in memory only; images are written as .bmp files so they can be dragged out, and the
//! folder is emptied on every start. Content that password managers mark as private is skipped.

use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;

pub const DEFAULT_LIMIT: u32 = 10;
pub const MIN_LIMIT: u32 = 5;
pub const MAX_LIMIT: u32 = 50;
const MAX_TEXT: usize = 20_000;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const BMP_HEADER: usize = 14;

#[derive(Clone, Debug, PartialEq)]
pub enum Clip {
    Text(String),
    Image { path: PathBuf, w: u32, h: u32 },
}

pub struct Clips {
    pub items: Vec<Clip>,
    next: u64,
    /// How many items to keep; older ones drop off as new ones arrive.
    limit: usize,
}

impl Clips {
    pub fn new(limit: u32) -> Self {
        Clips { items: Vec::new(), next: 0, limit: limit.max(1) as usize }
    }

    pub fn set_limit(&mut self, limit: u32) {
        self.limit = limit.max(1) as usize;
        self.trim();
    }

    pub fn push_text(&mut self, text: &str) -> bool {
        let text: String = text.trim().chars().take(MAX_TEXT).collect();
        if text.is_empty() {
            return false;
        }
        // Copying the same text again moves it to the front instead of adding a duplicate.
        self.items.retain(|c| *c != Clip::Text(text.clone()));
        self.items.insert(0, Clip::Text(text));
        self.trim();
        true
    }

    fn push_image(&mut self, dib: &[u8]) -> bool {
        let Some((w, h, bmp)) = bmp_from_dib(dib) else { return false };
        let dir = dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return false;
        }
        self.next += 1;
        let path = dir.join(format!("clip-{}.bmp", self.next));
        if let Err(e) = std::fs::write(&path, bmp) {
            crate::log(&format!("cannot keep a copied image: {e}"));
            return false;
        }
        self.items.insert(0, Clip::Image { path, w, h });
        self.trim();
        true
    }

    fn trim(&mut self) {
        for old in self.items.drain(self.limit.min(self.items.len())..) {
            if let Clip::Image { path, .. } = old {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        let _ = std::fs::remove_dir_all(dir());
    }
}

/// A one-line description for the copy overlay: (is an image, preview text).
pub fn summary(clip: &Clip) -> (bool, String) {
    match clip {
        Clip::Text(t) => (false, t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect()),
        Clip::Image { w, h, .. } => (true, format!("{w} × {h}")),
    }
}

pub fn dir() -> PathBuf {
    crate::config::data_dir().join("clips")
}

/// Starts watching the clipboard and drops images left over from the last run.
pub fn start(hwnd: HWND) {
    let _ = std::fs::remove_dir_all(dir());
    unsafe {
        if let Err(e) = AddClipboardFormatListener(hwnd) {
            crate::log(&format!("clipboard history unavailable: {e}"));
        }
    }
}

/// The clipboard changed: keep its text or image. Returns true when the history changed.
pub fn capture(clips: &mut Clips, hwnd: HWND) -> bool {
    match read(hwnd) {
        Some(Captured::Text(t)) => clips.push_text(&t),
        Some(Captured::Image(dib)) => clips.push_image(&dib),
        None => false,
    }
}

enum Captured {
    Text(String),
    Image(Vec<u8>),
}

fn format_id(name: PCWSTR) -> u32 {
    unsafe { RegisterClipboardFormatW(name) }
}

fn read(hwnd: HWND) -> Option<Captured> {
    unsafe {
        // Our own copies (from the shelf or this history) are already here.
        if GetClipboardOwner().is_ok_and(|o| o == hwnd) {
            return None;
        }
        for private in [w!("ExcludeClipboardContentFromMonitorProcessing"), w!("Clipboard Viewer Ignore")] {
            if IsClipboardFormatAvailable(format_id(private)).is_ok() {
                return None;
            }
        }
        // Another app may still hold the clipboard for a moment.
        if OpenClipboard(Some(hwnd)).is_err() {
            std::thread::sleep(std::time::Duration::from_millis(20));
            OpenClipboard(Some(hwnd)).ok()?;
        }
        let result = (|| {
            let history = format_id(w!("CanIncludeInClipboardHistory"));
            if let Some(bytes) = data(history) {
                if bytes.len() >= 4 && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == 0 {
                    return None;
                }
            }
            if let Some(bytes) = data(CF_UNICODETEXT) {
                let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                return Some(Captured::Text(String::from_utf16_lossy(&wide)));
            }
            data(CF_DIB).filter(|d| d.len() <= MAX_IMAGE_BYTES).map(Captured::Image)
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Bytes of one clipboard format; the clipboard must be open.
unsafe fn data(format: u32) -> Option<Vec<u8>> {
    if IsClipboardFormatAvailable(format).is_err() {
        return None;
    }
    let h = GetClipboardData(format).ok()?;
    let mem = HGLOBAL(h.0);
    let size = GlobalSize(mem);
    let ptr = GlobalLock(mem) as *const u8;
    if ptr.is_null() || size == 0 {
        return None;
    }
    let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
    let _ = GlobalUnlock(mem);
    Some(bytes)
}

/// Wraps a packed DIB in a .bmp file header; returns (width, height, file bytes).
fn bmp_from_dib(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u32_at = |o: usize| dib.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = u32_at(0)? as usize;
    if header < 40 || dib.len() <= header {
        return None;
    }
    let w = (u32_at(4)? as i32).unsigned_abs();
    let h = (u32_at(8)? as i32).unsigned_abs();
    let bits = u16::from_le_bytes([*dib.get(14)?, *dib.get(15)?]) as u32;
    let compression = u32_at(16)?;
    let used = u32_at(32)?;
    // BI_BITFIELDS (3) with a plain header keeps three colour masks after it.
    let masks = if header == 40 && compression == 3 { 12 } else { 0 };
    let colors = if used > 0 { used } else if bits <= 8 { 1 << bits } else { 0 } as usize;
    let offset = BMP_HEADER + header + masks + colors * 4;
    let mut out = Vec::with_capacity(BMP_HEADER + dib.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((BMP_HEADER + dib.len()) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some((w, h, out))
}

/// Puts a clip back on the clipboard. Our window becomes the owner, so it isn't captured again.
pub fn copy_back(hwnd: HWND, clip: &Clip) -> bool {
    match clip {
        Clip::Text(t) => crate::sys::copy_text(hwnd, t),
        Clip::Image { path, .. } => copy_bmp(hwnd, path),
    }
}

fn copy_bmp(hwnd: HWND, path: &Path) -> bool {
    let Ok(file) = std::fs::read(path) else { return false };
    let Some(dib) = file.get(BMP_HEADER..) else { return false };
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = (|| -> windows::core::Result<()> {
            let mem = GlobalAlloc(GMEM_MOVEABLE, dib.len())?;
            let ptr = GlobalLock(mem) as *mut u8;
            if ptr.is_null() {
                return Err(windows::core::Error::from_win32());
            }
            std::ptr::copy_nonoverlapping(dib.as_ptr(), ptr, dib.len());
            let _ = GlobalUnlock(mem);
            SetClipboardData(CF_DIB, Some(HANDLE(mem.0)))?;
            Ok(())
        })()
        .is_ok();
        let _ = CloseClipboard();
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_history_dedupes_and_keeps_the_newest_ten() {
        let mut c = Clips::new(DEFAULT_LIMIT);
        assert!(!c.push_text("   "));
        for i in 0..12 {
            c.push_text(&format!("item {i}"));
        }
        c.push_text("item 5");
        assert_eq!(c.items.len(), DEFAULT_LIMIT as usize);
        assert_eq!(c.items[0], Clip::Text("item 5".into()));
        assert_eq!(c.items.iter().filter(|i| **i == Clip::Text("item 5".into())).count(), 1);
    }

    #[test]
    fn a_smaller_limit_drops_the_oldest_items() {
        let mut c = Clips::new(10);
        for i in 0..8 {
            c.push_text(&format!("item {i}"));
        }
        c.set_limit(5);
        assert_eq!(c.items.len(), 5);
        assert_eq!(c.items[0], Clip::Text("item 7".into()));
        assert_eq!(c.items[4], Clip::Text("item 3".into()));
    }

    #[test]
    fn summary_flattens_text_and_sizes_images() {
        let text = Clip::Text("first line\n  second\tline".into());
        assert_eq!(summary(&text), (false, "first line second line".into()));
        let image = Clip::Image { path: PathBuf::from("x.bmp"), w: 1920, h: 1080 };
        assert_eq!(summary(&image), (true, "1920 × 1080".into()));
    }

    #[test]
    fn dib_gets_a_bitmap_file_header() {
        // 2x1 pixels, 32 bits, BI_RGB.
        let mut dib = vec![0u8; 40 + 8];
        dib[0] = 40;
        dib[4] = 2;
        dib[8] = 1;
        dib[12] = 1;
        dib[14] = 32;
        let (w, h, bmp) = bmp_from_dib(&dib).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes([bmp[2], bmp[3], bmp[4], bmp[5]]) as usize, 14 + dib.len());
        assert_eq!(u32::from_le_bytes([bmp[10], bmp[11], bmp[12], bmp[13]]), 54);
        assert!(bmp_from_dib(&dib[..20]).is_none());
    }
}

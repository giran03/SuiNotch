//! Files that exist only inside the dragging app, such as an image dragged out of a browser: the
//! drop carries their bytes (a "virtual file") instead of a path. They are saved into SuiNotch's own
//! folder so the shelf can keep them like any other file; nothing is downloaded.

use std::io::Write;
use std::path::{Path, PathBuf};

use windows::core::w;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::ReleaseStgMedium;
use windows::Win32::UI::Shell::{FILEDESCRIPTORW, FILEGROUPDESCRIPTORW};

use crate::store::{Shelf, ShelfItem};

/// A dropped file larger than this is not kept.
const MAX_BYTES: u64 = 50 * 1024 * 1024;
/// Most files taken from one drop.
const MAX_FILES: usize = 50;
const NAME_MAX_CHARS: usize = 100;
const DIRECTORY: u32 = 0x10;
const DIR_NAME: &str = "shelf_files";
/// How many times, 50 ms apart, to wait for a browser to finish writing its temporary file.
const WRITE_WAIT_TRIES: u32 = 20;

fn dir() -> PathBuf {
    crate::config::data_dir().join(DIR_NAME)
}

fn descriptor_format() -> u16 {
    unsafe { RegisterClipboardFormatW(w!("FileGroupDescriptorW")) as u16 }
}

fn contents_format() -> u16 {
    unsafe { RegisterClipboardFormatW(w!("FileContents")) as u16 }
}

fn descriptor_request() -> FORMATETC {
    FORMATETC { cfFormat: descriptor_format(), ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

/// Whether the drop carries files by content.
pub fn offers(data: &IDataObject) -> bool {
    unsafe { data.QueryGetData(&descriptor_request()).is_ok() }
}

/// Saves every file the drop carries by content and returns their paths; empty when it has none.
pub fn save(data: &IDataObject) -> Vec<PathBuf> {
    let names = names(data);
    if names.is_empty() {
        return Vec::new();
    }
    let folder = dir();
    if std::fs::create_dir_all(&folder).is_err() {
        return Vec::new();
    }
    let mut saved = Vec::new();
    for (index, name) in names {
        let path = unique_path(&folder, &safe_name(&name));
        match write_contents(data, index, &path) {
            Some(()) => saved.push(path),
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    saved
}

/// Files a browser drops are often its own temporary ones, deleted once the drop is over, so those are
/// copied into the shelf's folder now. Other files are kept where they are.
pub fn keep_transient(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let temp = std::env::temp_dir();
    paths.into_iter().map(|p| if is_under(&p, &temp) { copy_in(&p).unwrap_or(p) } else { p }).collect()
}

fn is_under(path: &Path, root: &Path) -> bool {
    let lower = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    let (path, root) = (lower(path), lower(root));
    let root = root.trim_end_matches('\\');
    path.len() > root.len() && path.starts_with(root) && path[root.len()..].starts_with('\\')
}

/// The file is copied once the browser has finished writing it, which can take a moment.
fn copy_in(source: &Path) -> Option<PathBuf> {
    let folder = dir();
    std::fs::create_dir_all(&folder).ok()?;
    for _ in 0..WRITE_WAIT_TRIES {
        if std::fs::metadata(source).is_ok_and(|m| m.is_file() && m.len() > 0 && m.len() <= MAX_BYTES) {
            let mut name = safe_name(&source.file_name()?.to_string_lossy());
            if !name.contains('.') {
                let mut head = [0u8; 12];
                let read = std::fs::File::open(source).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
                name.push_str(image_extension(&head[..read]));
            }
            let target = unique_path(&folder, &name);
            if std::fs::copy(source, &target).is_ok() {
                return Some(target);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

/// The extension for an image's first bytes; empty when they are not a common picture.
fn image_extension(head: &[u8]) -> &'static str {
    if head.starts_with(&[0x89, b'P', b'N', b'G']) {
        ".png"
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ".jpg"
    } else if head.starts_with(b"GIF8") {
        ".gif"
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        ".webp"
    } else {
        ""
    }
}

/// (index, file name) of each file in the drop, folders left out.
fn names(data: &IDataObject) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(mut medium) = data.GetData(&descriptor_request()) else { return out };
        let ptr = GlobalLock(medium.u.hGlobal) as *const FILEGROUPDESCRIPTORW;
        if !ptr.is_null() {
            let count = std::ptr::read_unaligned(std::ptr::addr_of!((*ptr).cItems)) as usize;
            let first = std::ptr::addr_of!((*ptr).fgd) as *const FILEDESCRIPTORW;
            let room = GlobalSize(medium.u.hGlobal).saturating_sub(std::mem::size_of::<u32>()) / std::mem::size_of::<FILEDESCRIPTORW>();
            for i in 0..count.min(room).min(MAX_FILES) {
                let d = std::ptr::read_unaligned(first.add(i));
                let (attributes, raw) = (d.dwFileAttributes, d.cFileName);
                if attributes & DIRECTORY != 0 {
                    continue;
                }
                let len = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                out.push((i as i32, String::from_utf16_lossy(&raw[..len])));
            }
            let _ = GlobalUnlock(medium.u.hGlobal);
        }
        ReleaseStgMedium(&mut medium);
    }
    out
}

/// Copies file `index` of the drop into `path`. None when it cannot be read or is too large.
fn write_contents(data: &IDataObject, index: i32, path: &Path) -> Option<()> {
    let f = FORMATETC {
        cfFormat: contents_format(),
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: index,
        tymed: (TYMED_ISTREAM.0 | TYMED_HGLOBAL.0) as u32,
    };
    let mut file = std::fs::File::create(path).ok()?;
    unsafe {
        let mut medium = data.GetData(&f).ok()?;
        let done = if medium.tymed == TYMED_ISTREAM.0 as u32 {
            match (*medium.u.pstm).as_ref() {
                Some(stream) => copy_stream(stream, &mut file),
                None => None,
            }
        } else if medium.tymed == TYMED_HGLOBAL.0 as u32 {
            copy_global(medium.u.hGlobal, &mut file)
        } else {
            None
        };
        ReleaseStgMedium(&mut medium);
        done
    }
}

unsafe fn copy_stream(stream: &IStream, out: &mut impl Write) -> Option<()> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let mut read = 0u32;
        stream.Read(buffer.as_mut_ptr() as *mut _, buffer.len() as u32, Some(&mut read)).ok().ok()?;
        if read == 0 {
            return Some(());
        }
        total += read as u64;
        if total > MAX_BYTES {
            return None;
        }
        out.write_all(&buffer[..read as usize]).ok()?;
    }
}

unsafe fn copy_global(mem: windows::Win32::Foundation::HGLOBAL, out: &mut impl Write) -> Option<()> {
    let size = GlobalSize(mem);
    if size as u64 > MAX_BYTES {
        return None;
    }
    let ptr = GlobalLock(mem) as *const u8;
    if ptr.is_null() {
        return None;
    }
    let result = out.write_all(std::slice::from_raw_parts(ptr, size)).ok();
    let _ = GlobalUnlock(mem);
    result
}

/// A file name that is safe to create: no folders, no characters Windows refuses, not a device name.
fn safe_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == ' ');
    if cleaned.is_empty() {
        return "dropped".to_string();
    }
    let (stem, ext) = match cleaned.rfind('.') {
        Some(i) if i > 0 => (&cleaned[..i], &cleaned[i..]),
        _ => (cleaned, ""),
    };
    let mut stem: String = stem.chars().take(NAME_MAX_CHARS).collect();
    let ext: String = ext.chars().take(16).collect();
    const DEVICES: [&str; 6] = ["con", "prn", "aux", "nul", "com1", "lpt1"];
    if DEVICES.contains(&stem.to_lowercase().as_str()) {
        stem.insert(0, '_');
    }
    format!("{stem}{ext}")
}

/// `name` inside `folder`, or `name (2)`, `name (3)`... when that is taken.
fn unique_path(folder: &Path, name: &str) -> PathBuf {
    let first = folder.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| folder.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Deletes saved files the shelf no longer holds. Run at start and when the shelf is cleared, not
/// on every removal, since an app that was handed one by a drag may still be reading it.
pub fn sweep(shelf: &Shelf) {
    let Ok(entries) = std::fs::read_dir(dir()) else { return };
    let kept: Vec<&PathBuf> = shelf.items.iter().filter_map(|i| if let ShelfItem::File { path } = i { Some(path) } else { None }).collect();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && !kept.contains(&&path) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_lose_folders_and_forbidden_characters() {
        assert_eq!(safe_name("cat.png"), "cat.png");
        assert_eq!(safe_name("..\\..\\evil.exe"), "evil.exe");
        assert_eq!(safe_name("a/b/c.jpg"), "c.jpg");
        assert_eq!(safe_name("what?*<>.png"), "what____.png");
    }

    #[test]
    fn empty_and_device_names_are_replaced() {
        assert_eq!(safe_name(""), "dropped");
        assert_eq!(safe_name("..."), "dropped");
        assert_eq!(safe_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_name(".hidden"), "hidden");
    }

    #[test]
    fn long_names_are_shortened_and_keep_their_extension() {
        let name = safe_name(&format!("{}.webp", "x".repeat(300)));
        assert_eq!(name.chars().count(), NAME_MAX_CHARS + 5);
        assert!(name.ends_with(".webp"));
    }

    #[test]
    fn only_files_inside_the_temp_folder_count_as_transient() {
        let temp = Path::new("C:\\Users\\a\\AppData\\Local\\Temp");
        assert!(is_under(Path::new("c:/users/a/appdata/local/temp/x/img"), temp));
        assert!(!is_under(Path::new("C:\\Users\\a\\Pictures\\img.png"), temp));
        assert!(!is_under(Path::new("C:\\Users\\a\\AppData\\Local\\Temporary\\img"), temp));
        assert!(!is_under(temp, temp));
    }

    #[test]
    fn pictures_are_recognised_by_their_first_bytes() {
        assert_eq!(image_extension(&[0x89, b'P', b'N', b'G', 0, 0]), ".png");
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), ".jpg");
        assert_eq!(image_extension(b"GIF89a"), ".gif");
        assert_eq!(image_extension(b"RIFF\0\0\0\0WEBPVP8 "), ".webp");
        assert_eq!(image_extension(b"hello"), "");
    }

    #[test]
    fn taken_names_get_a_number() {
        let folder = std::env::temp_dir().join(format!("suinotch-dropped-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(unique_path(&folder, "a.png"), folder.join("a.png"));
        std::fs::write(folder.join("a.png"), b"x").unwrap();
        assert_eq!(unique_path(&folder, "a.png"), folder.join("a (2).png"));
        std::fs::write(folder.join("a (2).png"), b"x").unwrap();
        assert_eq!(unique_path(&folder, "a.png"), folder.join("a (3).png"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}

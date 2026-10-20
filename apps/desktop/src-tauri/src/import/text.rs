//! Reading the files other clients save: their text encodings, INI files, and folders of
//! session files.

use std::io::Read;
use std::path::{Path, PathBuf};

use encoding_rs::Encoding;

use crate::error::{AppError, AppResult};

/// The largest file an import reads. Session files and configs are far smaller.
pub(super) const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// The most session files an import reads from a folder or an archive.
pub(super) const MAX_FILES: usize = 10_000;
/// How deep an import looks into the subfolders of a sessions folder.
pub(super) const MAX_DEPTH: usize = 16;

/// Reads a file of at most [`MAX_FILE_BYTES`].
pub(super) fn read_file(path: &Path) -> AppResult<Vec<u8>> {
    read_file_up_to(path, MAX_FILE_BYTES)
}

/// Reads a file of at most `max` bytes.
pub(super) fn read_file_up_to(path: &Path, max: u64) -> AppResult<Vec<u8>> {
    let file = std::fs::File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::not_found(&path.display().to_string())
        } else {
            AppError::io(e)
        }
    })?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(AppError::io)?;
    if bytes.len() as u64 > max {
        return Err(AppError::io(format!("{} is too large", path.display())));
    }
    Ok(bytes)
}

/// Decodes the text of a file another app saved: UTF-8 or UTF-16 with a byte order mark, UTF-16
/// or UTF-8 without one, and otherwise the Windows ANSI code page, which older Windows apps write.
pub(super) fn decode(bytes: &[u8]) -> String {
    decode_with(bytes, ansi())
}

/// [`decode`] with the given ANSI code page.
pub(super) fn decode_with(bytes: &[u8], ansi: &'static Encoding) -> String {
    if let Some((encoding, bom)) = Encoding::for_bom(bytes) {
        return encoding
            .decode_without_bom_handling(&bytes[bom..])
            .0
            .into_owned();
    }
    if let Some(encoding) = utf16_without_bom(bytes) {
        return encoding.decode_without_bom_handling(bytes).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => ansi.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

/// UTF-16 without a byte order mark, told by the zero byte of each ASCII character, which session
/// files are mostly made of: text in UTF-8 or an ANSI code page has no zero bytes. A few characters
/// such as 一 (U+4E00) have a zero byte on the other side.
fn utf16_without_bom(bytes: &[u8]) -> Option<&'static Encoding> {
    let (pairs, rest) = bytes.as_chunks::<2>();
    if pairs.len() < 2 || !rest.is_empty() {
        return None;
    }
    let (first, second) = pairs.iter().fold((0, 0), |(first, second), pair| {
        (
            first + usize::from(pair[0] == 0),
            second + usize::from(pair[1] == 0),
        )
    });
    let half = bytes.len() / 4;
    if second > half && first < second / 8 {
        Some(encoding_rs::UTF_16LE)
    } else if first > half && second < first / 8 {
        Some(encoding_rs::UTF_16BE)
    } else {
        None
    }
}

/// The Windows ANSI code page of this device.
#[cfg(windows)]
pub(super) fn ansi() -> &'static Encoding {
    // SAFETY: GetACP takes no arguments and only reads the system's setting.
    code_page(unsafe { windows::Win32::Globalization::GetACP() })
}

/// Windows-1252, the ANSI code page of Western Windows, for files copied from Windows.
#[cfg(not(windows))]
pub(super) fn ansi() -> &'static Encoding {
    encoding_rs::WINDOWS_1252
}

/// The encoding of a Windows ANSI code page; Windows-1252 for one this does not know.
#[cfg(any(windows, test))]
fn code_page(cp: u32) -> &'static Encoding {
    match cp {
        874 => encoding_rs::WINDOWS_874,
        932 => encoding_rs::SHIFT_JIS,
        936 => encoding_rs::GBK,
        949 => encoding_rs::EUC_KR,
        950 => encoding_rs::BIG5,
        1250 => encoding_rs::WINDOWS_1250,
        1251 => encoding_rs::WINDOWS_1251,
        1253 => encoding_rs::WINDOWS_1253,
        1254 => encoding_rs::WINDOWS_1254,
        1255 => encoding_rs::WINDOWS_1255,
        1256 => encoding_rs::WINDOWS_1256,
        1257 => encoding_rs::WINDOWS_1257,
        1258 => encoding_rs::WINDOWS_1258,
        54936 => encoding_rs::GB18030,
        65001 => encoding_rs::UTF_8,
        _ => encoding_rs::WINDOWS_1252,
    }
}

/// One `[section]` of an INI file, with its `key=value` lines in order.
pub(super) struct Section {
    pub name: String,
    pub entries: Vec<(String, String)>,
}

impl Section {
    /// The value of `key`, compared without case, trimmed.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// The sections of an INI file in order. Lines before the first section header, comments
/// (`;`) and lines without `=` are left out; keys and values are trimmed.
pub(super) fn ini(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push(Section {
                name: name.trim().to_owned(),
                entries: Vec::new(),
            });
        } else if let (Some(section), Some((key, value))) =
            (sections.last_mut(), line.split_once('='))
        {
            section
                .entries
                .push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    sections
}

/// The section called `name`, compared without case.
pub(super) fn section<'a>(sections: &'a [Section], name: &str) -> Option<&'a Section> {
    sections.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// The files under `root` in name order, each with the folders between, outermost first: at
/// most [`MAX_FILES`] of them, `depth` folders deep. Links are not followed.
pub(super) fn files_under(root: &Path, depth: usize) -> AppResult<Vec<(PathBuf, Vec<String>)>> {
    let mut files = Vec::new();
    walk(root, depth, &mut Vec::new(), &mut files)?;
    Ok(files)
}

fn walk(
    dir: &Path,
    depth: usize,
    folders: &mut Vec<String>,
    files: &mut Vec<(PathBuf, Vec<String>)>,
) -> AppResult<()> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map_err(AppError::io)?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && folders.len() < depth {
            folders.push(entry.file_name().to_string_lossy().into_owned());
            walk(&entry.path(), depth, folders, files)?;
            folders.pop();
        } else if kind.is_file() {
            if files.len() == MAX_FILES {
                return Err(AppError::io(format!(
                    "{} holds too many files",
                    dir.display()
                )));
            }
            files.push((entry.path(), folders.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_decodes_from_its_byte_order_mark_utf8_or_the_ansi_code_page() {
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(
                "[CONNECTION]\r\nHost=生产"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        assert_eq!(
            decode_with(&utf16, encoding_rs::GBK),
            "[CONNECTION]\r\nHost=生产"
        );
        // Without the byte order mark too, with a character whose low byte is zero.
        let bare: Vec<u8> = "[CONNECTION]\r\nHost=一号机"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            decode_with(&bare, encoding_rs::GBK),
            "[CONNECTION]\r\nHost=一号机"
        );
        let be: Vec<u8> = "Host=web"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(decode_with(&be, encoding_rs::GBK), "Host=web");
        assert_eq!(
            decode_with("\u{feff}a=1".as_bytes(), encoding_rs::GBK),
            "a=1"
        );
        assert_eq!(decode_with("组=1".as_bytes(), encoding_rs::GBK), "组=1");
        let (gbk, _, _) = encoding_rs::GBK.encode("SubRep=生产\\网关");
        assert_eq!(decode_with(&gbk, encoding_rs::GBK), "SubRep=生产\\网关");
        assert_eq!(decode_with(b"caf\xe9", encoding_rs::WINDOWS_1252), "café");
        assert_eq!(code_page(936), encoding_rs::GBK);
        assert_eq!(code_page(437), encoding_rs::WINDOWS_1252);
    }

    #[test]
    fn ini_keeps_sections_and_lines_in_order() {
        let sections = ini(
            "orphan=1\r\n[A]\r\n; comment\r\nkey = a=b \r\nnovalue\r\n[ b ]\r\nKey=2\r\nkey=3\r\n",
        );
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "A");
        assert_eq!(sections[0].get("KEY"), Some("a=b"));
        let b = section(&sections, "B").unwrap();
        assert_eq!(b.get("key"), Some("2"), "the first line wins");
        assert_eq!(b.entries.len(), 2);
    }

    #[test]
    fn folders_are_walked_in_name_order() {
        let root =
            std::env::temp_dir().join(format!("hatoba-import-walk-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(root.join("b").join("c")).unwrap();
        std::fs::write(root.join("z.xsh"), "").unwrap();
        std::fs::write(root.join("b").join("c").join("y.xsh"), "").unwrap();
        std::fs::write(root.join("a.xsh"), "").unwrap();
        let files = files_under(&root, MAX_DEPTH).unwrap();
        let top = files_under(&root, 0).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(top.len(), 2, "{top:?}");
        let names: Vec<(String, Vec<String>)> = files
            .into_iter()
            .map(|(p, f)| (p.file_name().unwrap().to_string_lossy().into_owned(), f))
            .collect();
        assert_eq!(
            names,
            [
                ("a.xsh".to_owned(), vec![]),
                ("y.xsh".to_owned(), vec!["b".to_owned(), "c".to_owned()]),
                ("z.xsh".to_owned(), vec![]),
            ]
        );
    }
}

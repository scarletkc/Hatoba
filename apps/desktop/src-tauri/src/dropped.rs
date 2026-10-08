//! Text files dropped on the AI panel in the desktop app (AI-35). The app's webview takes file
//! drops itself (the SFTP panel's uploads use the paths it hands over), so the WebView gets paths
//! it cannot read. Rust records the paths of the window's most recent drop from the window's own
//! drag-and-drop event, and `ai_read_dropped_files` reads only those, once each, so the command
//! cannot read any other file. A file is read with the rules of the panel's File API path
//! (`features/ai/attachments.ts`): UTF-8 text of at most 256 KB, images refused, and only its base
//! name goes back, never its path.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::dto::{DroppedFile, DroppedFileRefusal};

/// The largest file attached, in bytes (AI-35).
pub const FILE_MAX_BYTES: u64 = 256 * 1024;

/// Extensions of images, which attachments do not take yet (the panel's `IMAGE_EXT`).
const IMAGE_EXTENSIONS: [&str; 13] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "heic", "heif", "avif", "tif", "tiff",
];

/// The paths of the window's most recent file drop that have not been read yet.
#[derive(Default)]
pub struct DroppedPaths(Mutex<Vec<PathBuf>>);

impl DroppedPaths {
    /// A drop on the window: its paths replace those of the drop before.
    pub fn record(&self, paths: &[PathBuf]) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = paths.to_vec();
    }

    /// The recorded paths `requested` names, which are then forgotten (each is read once).
    /// `None`, and nothing forgotten, when any of them is not a path of the recorded drop.
    pub fn take(&self, requested: &[String]) -> Option<Vec<PathBuf>> {
        let mut recorded = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let wanted: Vec<PathBuf> = requested.iter().map(PathBuf::from).collect();
        if !wanted.iter().all(|p| recorded.contains(p)) {
            return None;
        }
        recorded.retain(|p| !wanted.contains(p));
        Some(wanted)
    }
}

/// What follows the last `/` or `\` of the file's name, as the panel names files.
pub fn base_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn is_image(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| {
        IMAGE_EXTENSIONS
            .iter()
            .any(|image| image.eq_ignore_ascii_case(ext))
    })
}

/// Reads a dropped file as an attachment, or says why it is refused.
pub fn read(path: &Path) -> DroppedFile {
    let name = base_name(path);
    let refused = |reason| DroppedFile::Refused {
        name: name.clone(),
        reason,
    };
    if is_image(&name) {
        return refused(DroppedFileRefusal::Image);
    }
    let Ok(file) = std::fs::File::open(path) else {
        return refused(DroppedFileRefusal::Unreadable);
    };
    match file.metadata() {
        Ok(meta) if !meta.is_file() => return refused(DroppedFileRefusal::Unreadable),
        Ok(meta) if meta.len() > FILE_MAX_BYTES => return refused(DroppedFileRefusal::TooLarge),
        Ok(_) => {}
        Err(_) => return refused(DroppedFileRefusal::Unreadable),
    }
    // One byte more than the limit, in case the file grew since its size was read.
    let mut bytes = Vec::new();
    if file
        .take(FILE_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return refused(DroppedFileRefusal::Unreadable);
    }
    if bytes.len() as u64 > FILE_MAX_BYTES {
        return refused(DroppedFileRefusal::TooLarge);
    }
    if bytes.contains(&0) {
        return refused(DroppedFileRefusal::Binary);
    }
    match String::from_utf8(bytes) {
        Ok(text) => DroppedFile::Ok {
            // Like the WebView's `TextDecoder`, which drops a byte order mark.
            text: text
                .strip_prefix('\u{feff}')
                .map(str::to_owned)
                .unwrap_or(text),
            name,
        },
        Err(_) => refused(DroppedFileRefusal::Binary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system's temporary directory, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "hatoba-dropped-{tag}-{}-{}",
                std::process::id(),
                uuid::Uuid::now_v7()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ok(name: &str, text: &str) -> DroppedFile {
        DroppedFile::Ok {
            name: name.into(),
            text: text.into(),
        }
    }

    fn refused(name: &str, reason: DroppedFileRefusal) -> DroppedFile {
        DroppedFile::Refused {
            name: name.into(),
            reason,
        }
    }

    fn strings(paths: &[&PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn only_paths_of_the_last_drop_are_read_and_each_once() {
        let dir = TempDir::new("record");
        let a = dir.file("a.txt", b"a");
        let b = dir.file("b.txt", b"b");
        let elsewhere = dir.file("secret.txt", b"not dropped");
        let dropped = DroppedPaths::default();
        assert_eq!(dropped.take(&strings(&[&a])), None, "nothing was dropped");

        dropped.record(&[a.clone(), b.clone()]);
        // A path that was not dropped refuses the whole request, which then takes nothing.
        assert_eq!(dropped.take(&strings(&[&a, &elsewhere])), None);
        assert_eq!(dropped.take(&strings(&[&a])), Some(vec![a.clone()]));
        assert_eq!(dropped.take(&strings(&[&a])), None, "read once");
        // A newer drop replaces the paths of the one before.
        dropped.record(std::slice::from_ref(&elsewhere));
        assert_eq!(dropped.take(&strings(&[&b])), None);
        assert_eq!(dropped.take(&[]), Some(Vec::new()));
    }

    #[test]
    fn files_are_read_with_the_panels_rules() {
        let dir = TempDir::new("read");
        let text = dir.file(
            "nginx.conf",
            "server {\r\n  listen 80;\r\n}\n日本語".as_bytes(),
        );
        assert_eq!(
            read(&text),
            ok("nginx.conf", "server {\r\n  listen 80;\r\n}\n日本語")
        );
        let bom = dir.file("bom.md", b"\xef\xbb\xbf# Title");
        assert_eq!(read(&bom), ok("bom.md", "# Title"));
        let empty = dir.file("empty.txt", b"");
        assert_eq!(read(&empty), ok("empty.txt", ""));

        let at_limit = dir.file("limit.log", &vec![b'x'; 256 * 1024]);
        assert!(
            matches!(read(&at_limit), DroppedFile::Ok { text, .. } if text.len() == 256 * 1024)
        );
        let over = dir.file("big.log", &vec![b'x'; 256 * 1024 + 1]);
        assert_eq!(
            read(&over),
            refused("big.log", DroppedFileRefusal::TooLarge)
        );

        let nul = dir.file("data.bin", b"text\0more");
        assert_eq!(read(&nul), refused("data.bin", DroppedFileRefusal::Binary));
        let latin1 = dir.file("latin1.txt", b"caf\xe9");
        assert_eq!(
            read(&latin1),
            refused("latin1.txt", DroppedFileRefusal::Binary)
        );
        // An image is refused by its name, even when its bytes are text (an SVG).
        let svg = dir.file("Logo.SVG", b"<svg/>");
        assert_eq!(read(&svg), refused("Logo.SVG", DroppedFileRefusal::Image));

        let folder = dir.0.join("folder.d");
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(
            read(&folder),
            refused("folder.d", DroppedFileRefusal::Unreadable)
        );
        let gone = dir.0.join("gone.txt");
        assert_eq!(
            read(&gone),
            refused("gone.txt", DroppedFileRefusal::Unreadable)
        );
    }

    #[test]
    fn results_name_the_file_never_its_path() {
        let dir = TempDir::new("name");
        let path = dir.file("notes.txt", b"hello");
        let json = serde_json::to_string(&read(&path)).unwrap();
        assert_eq!(json, r#"{"status":"ok","name":"notes.txt","text":"hello"}"#);
        assert!(!json.contains(&*dir.0.to_string_lossy()));
        assert_eq!(base_name(Path::new("/home/someone/a.txt")), "a.txt");
        assert_eq!(base_name(Path::new("C:\\Users\\someone\\b.txt")), "b.txt");
    }
}

//! Streaming downloads, confined to the chosen folder; incomplete files never replace a file.
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
        && !matches!(
            name.split('.')
                .next()
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
}

struct Download {
    file: File,
    temporary: PathBuf,
    directory: PathBuf,
    name: String,
}

impl Drop for Download {
    fn drop(&mut self) {
        // Rust opens Windows files with FILE_SHARE_DELETE, allowing removal before the
        // handle closes. The file is deleted when the last open handle is dropped.
        let _ = fs::remove_file(&self.temporary);
    }
}

#[derive(Default)]
pub struct Downloads(Mutex<HashMap<String, Download>>);

impl Downloads {
    pub fn begin(&self, directory: &Path, name: &str, id: String) -> io::Result<String> {
        if !valid_name(name) || !directory.is_absolute() || !directory.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid download name or directory",
            ));
        }
        let mut all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if all.len() >= 8 {
            return Err(io::Error::other("too many ZMODEM downloads"));
        }
        let temporary = directory.join(format!(".hatoba-zmodem-{id}.part"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        all.insert(
            id.clone(),
            Download {
                file,
                temporary,
                directory: directory.to_owned(),
                name: name.to_owned(),
            },
        );
        Ok(id)
    }

    pub fn write(&self, id: &str, data: &[u8]) -> io::Result<()> {
        if data.len() > 256 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "download chunk too large",
            ));
        }
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "download ended"))?
            .file
            .write_all(data)
    }

    pub fn finish(&self, id: &str, commit: bool) -> io::Result<Option<String>> {
        let Some(download) = self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(id) else {
            return Ok(None);
        };
        if !commit {
            return Ok(None);
        }
        download.file.sync_all()?;
        let original = Path::new(&download.name);
        for n in 0..10000 {
            let name = if n == 0 {
                download.name.clone()
            } else {
                let stem = original.file_stem().unwrap_or_default().to_string_lossy();
                let extension = original
                    .extension()
                    .map(|s| format!(".{}", s.to_string_lossy()))
                    .unwrap_or_default();
                format!("{stem} ({n}){extension}")
            };
            let target = download.directory.join(name);
            // An atomic, no-clobber publication on the same filesystem. Never follow an
            // existing symlink or truncate an existing file, even if another process races us.
            match fs::hard_link(&download.temporary, &target) {
                Ok(()) => return Ok(Some(target.to_string_lossy().into_owned())),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("too many files with the same name"))
    }

    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_and_windows_special_names() {
        for name in ["../x", "a/b", "a\\b", "C:x", "NUL.txt", "x.", "a\n", ".."] {
            assert!(!valid_name(name), "{name}");
        }
        assert!(valid_name("测试文件.bin"));
    }
    #[test]
    fn streams_without_overwriting_and_removes_partial_files() {
        let dir = std::env::temp_dir().join(format!("hatoba-zmodem-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("test.bin"), b"original").unwrap();
        let files = Downloads::default();
        let id = files.begin(&dir, "test.bin", "one".into()).unwrap();
        files.write(&id, &[0, 255, 24]).unwrap();
        let path = files.finish(&id, true).unwrap().unwrap();
        assert_eq!(fs::read(path).unwrap(), [0, 255, 24]);
        assert_eq!(fs::read(dir.join("test.bin")).unwrap(), b"original");
        let id = files.begin(&dir, "cancel.bin", "two".into()).unwrap();
        files.write(&id, b"partial").unwrap();
        files.finish(&id, false).unwrap();
        assert!(!dir.join("cancel.bin").exists());
        assert!(!dir.join(".hatoba-zmodem-two.part").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}

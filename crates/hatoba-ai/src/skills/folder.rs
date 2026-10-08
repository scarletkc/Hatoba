//! Import from a folder on disk.
//!
//! The walk never follows a symbolic link or a junction, ignores everything that is neither a
//! file nor a folder, does not enter `.git` or `__MACOSX`, and is bounded in depth and in the
//! number of entries it looks at. A file is opened only after it was seen to be a regular file,
//! checked again right before opening and on the open handle, and read for at most 32 KB + 1.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use super::import::{Collector, MAX_PATH_DEPTH, Read, assemble, ignored_dir, shown_path};
use super::{MAX_FILES, SKILL_MD, SkillError, SkillImport, SkillIssue};

/// Directory entries looked at in one import, of any kind. Far above [`MAX_FILES`] so that
/// folders of empty directories or ignored files cannot keep the walk busy.
const MAX_WALK_ENTRIES: usize = 1_000;

fn io_error(err: std::io::Error) -> SkillError {
    SkillError::Io(err.to_string())
}

/// Reads the skill in the folder `path`: `SKILL.md` directly in it, or inside its single
/// subfolder that has one. The folder itself is the user's choice and may be a link; nothing
/// inside it is followed.
///
/// A folder with no `SKILL.md`, or with several subfolders that have one, is imported with
/// [`SkillIssue::MissingSkillMd`]. The folder not existing or not being readable is an error.
pub fn read_folder(path: &Path) -> Result<SkillImport, SkillError> {
    if !fs::metadata(path).map_err(io_error)?.is_dir() {
        return Err(SkillError::Io("not a folder".to_owned()));
    }
    let Some(dir) = locate_skill_dir(path)? else {
        return Ok(SkillImport {
            package: None,
            skipped: Vec::new(),
            issues: vec![SkillIssue::MissingSkillMd],
        });
    };

    // SKILL.md is added before the walk, so that stopping at a limit cannot lose it.
    let mut walk = Walk {
        collector: Collector::new(),
        seen: 0,
        stopped: false,
        too_many_entries: false,
    };
    let skill_md = dir.join(SKILL_MD);
    let declared = fs::symlink_metadata(&skill_md).map_err(io_error)?.len();
    walk.collector.add_file(SKILL_MD, declared, skill_md);
    walk.dir(&dir, "", 0)?;
    let (stopped, seen) = (walk.stopped, walk.seen);
    let too_many_entries = walk.too_many_entries;
    let (entries, mut issues) = walk.collector.finish();
    if too_many_entries {
        issues.push(SkillIssue::TooManyFiles(seen));
    }
    assemble(entries, issues, stopped, |file, limit| {
        read_file(file, limit)
    })
}

fn has_skill_md(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(SKILL_MD)).is_ok_and(|meta| meta.file_type().is_file())
}

/// `path` when it holds `SKILL.md`, else its only subfolder that does. A link named `SKILL.md`
/// does not count. A file system that ignores case also finds `skill.md`.
fn locate_skill_dir(path: &Path) -> Result<Option<PathBuf>, SkillError> {
    if has_skill_md(path) {
        return Ok(Some(path.to_owned()));
    }
    let mut found = None;
    for entry in fs::read_dir(path).map_err(io_error)?.take(MAX_WALK_ENTRIES) {
        let entry = entry.map_err(io_error)?;
        let is_dir = entry.file_type().map_err(io_error)?.is_dir();
        let ignored = entry.file_name().to_str().is_none_or(ignored_dir);
        if !is_dir || ignored || !has_skill_md(&entry.path()) {
            continue;
        }
        if found.is_some() {
            return Ok(None);
        }
        found = Some(entry.path());
    }
    Ok(found)
}

struct Walk {
    collector: Collector<PathBuf>,
    /// Directory entries looked at.
    seen: usize,
    /// The walk ended early at a limit.
    stopped: bool,
    /// The limit was [`MAX_WALK_ENTRIES`] (the file limit is reported by the import itself).
    too_many_entries: bool,
}

impl Walk {
    /// Lists the files of `dir`, `rel` being its path below the skill folder and `depth` the
    /// number of folders above it.
    fn dir(&mut self, dir: &Path, rel: &str, depth: usize) -> Result<(), SkillError> {
        let mut children = Vec::new();
        for entry in fs::read_dir(dir).map_err(io_error)? {
            self.seen += 1;
            if self.seen > MAX_WALK_ENTRIES {
                self.stopped = true;
                self.too_many_entries = true;
                return Ok(());
            }
            children.push(entry.map_err(io_error)?);
        }
        children.sort_by_key(fs::DirEntry::file_name);

        for child in children {
            let kind = child.file_type().map_err(io_error)?;
            let file_name = child.file_name();
            let Some(name) = file_name.to_str() else {
                self.collector.add_unsafe(&file_name.to_string_lossy());
                continue;
            };
            let child_rel = if rel.is_empty() {
                name.to_owned()
            } else {
                format!("{rel}/{name}")
            };
            if kind.is_dir() {
                if ignored_dir(name) {
                    continue;
                }
                if depth + 2 > MAX_PATH_DEPTH {
                    self.collector.add_unsafe(&shown_path(&child_rel));
                    continue;
                }
                self.dir(&child.path(), &child_rel, depth + 1)?;
            } else if kind.is_file() && !child_rel.eq_ignore_ascii_case(SKILL_MD) {
                let declared = child.metadata().map_err(io_error)?.len();
                self.collector.add_file(&child_rel, declared, child.path());
                if self.collector.len() > MAX_FILES {
                    self.stopped = true;
                }
            }
            // Symbolic links, junctions, pipes, sockets and devices are none of the above.
            if self.stopped {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// At most `limit` bytes of a regular file.
fn read_file(path: &Path, limit: u64) -> Result<Read, SkillError> {
    let changed = || SkillError::Io("a file changed while it was read".to_owned());
    if !fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file()) {
        return Err(changed());
    }
    let file = fs::File::open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(changed());
    }
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).map_err(io_error)?;
    Ok(Read::Bytes(bytes))
}

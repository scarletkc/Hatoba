//! Import from and export to a `.zip`.
//!
//! Nothing is extracted: entries are listed from the central directory and read into memory, at
//! most 32 KB + 1 of each, so a bomb that claims a small size and inflates to gigabytes (or the
//! reverse) costs a bounded amount of work. Symbolic link entries and directories are ignored.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Cursor, Read as _, Write as _};
use std::path::Path;

use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::import::{Collector, Entry, Read, assemble};
use super::{
    SKILL_MD, SkillError, SkillFileData, SkillImport, SkillIssue, SkillPackage, render_skill_md,
    validate,
};

/// The largest `.zip` file read. A skill is at most [`MAX_TOTAL_BYTES`] of text, which a `.zip`
/// stores in about that much or less; the rest is room for junk entries.
///
/// [`MAX_TOTAL_BYTES`]: super::MAX_TOTAL_BYTES
pub(super) const MAX_ZIP_BYTES: u64 = 16 * 1024 * 1024;
/// Entries in the central directory, of any kind (directories and junk included).
const MAX_ZIP_ENTRIES: usize = 2_000;

fn zip_error(err: ZipError) -> SkillError {
    SkillError::Zip(err.to_string())
}

fn io_error(err: std::io::Error) -> SkillError {
    SkillError::Io(err.to_string())
}

/// Reads the skill in the `.zip` at `path`: `SKILL.md` at the root of the archive, or inside its
/// single top-level folder that has one. Entries outside that folder are not part of the skill.
///
/// An archive with no `SKILL.md`, or with several top-level folders that have one, is imported
/// with [`SkillIssue::MissingSkillMd`]. A file that cannot be opened is [`SkillError::Io`]; one
/// that is not a readable archive, a corrupt entry, or an archive over 16 MB is
/// [`SkillError::Zip`]. An entry other than `SKILL.md` that uses an unsupported compression
/// method or encryption is skipped like a file that is not text.
pub fn read_zip(path: &Path) -> Result<SkillImport, SkillError> {
    let mut data = Vec::new();
    fs::File::open(path)
        .map_err(io_error)?
        .take(MAX_ZIP_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(io_error)?;
    if data.len() as u64 > MAX_ZIP_BYTES {
        return Err(SkillError::Zip(
            "the .zip file is larger than 16 MB".to_owned(),
        ));
    }
    let mut archive = ZipArchive::new(Cursor::new(data)).map_err(zip_error)?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Ok(SkillImport {
            package: None,
            skipped: Vec::new(),
            issues: vec![SkillIssue::TooManyFiles(archive.len())],
        });
    }

    let mut collector = Collector::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(zip_error)?;
        if !entry.is_dir() && !entry.is_symlink() {
            collector.add_file(entry.name(), entry.size(), index);
        }
    }
    let (entries, issues) = collector.finish();
    let Some(entries) = within_skill_folder(entries) else {
        let mut issues = issues;
        issues.push(SkillIssue::MissingSkillMd);
        return Ok(SkillImport {
            package: None,
            skipped: Vec::new(),
            issues,
        });
    };

    assemble(entries, issues, false, |&index, limit| {
        let entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(
                ZipError::UnsupportedArchive(_)
                | ZipError::CompressionMethodNotSupported(_)
                | ZipError::InvalidPassword,
            ) => return Ok(Read::Unreadable),
            Err(err) => return Err(zip_error(err)),
        };
        let mut bytes = Vec::new();
        entry
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(|err| SkillError::Zip(err.to_string()))?;
        Ok(Read::Bytes(bytes))
    })
}

/// The entries below the skill's folder, with that folder removed from their paths: the
/// archive root when it has `SKILL.md`, else the only top-level folder that has one.
fn within_skill_folder(entries: Vec<Entry<usize>>) -> Option<Vec<Entry<usize>>> {
    let prefix = if entries.iter().any(|e| e.path == SKILL_MD) {
        String::new()
    } else {
        let folders: BTreeSet<&str> = entries
            .iter()
            .filter_map(|e| e.path.strip_suffix("/SKILL.md"))
            .filter(|folder| !folder.contains('/'))
            .collect();
        let mut folders = folders.into_iter();
        let (Some(folder), None) = (folders.next(), folders.next()) else {
            return None;
        };
        format!("{folder}/")
    };
    Some(
        entries
            .into_iter()
            .filter_map(|e| {
                let path = e.path.strip_prefix(&prefix)?.to_owned();
                Some(Entry { path, ..e })
            })
            .collect(),
    )
}

/// Writes `pkg` as a `.zip` at `path`: `<name>/SKILL.md` (frontmatter rendered by
/// [`render_skill_md`]) and `<name>/<path>` for every file, sorted by path, deflated, with fixed
/// timestamps so the same skill gives the same bytes.
///
/// A skill whose name or file paths are not safe is not exported ([`SkillError::Zip`]); other
/// limits (sizes, description length) do not stop an export of what the user already has.
pub fn write_zip(pkg: &SkillPackage, path: &Path) -> Result<(), SkillError> {
    for issue in validate(pkg) {
        match issue {
            SkillIssue::InvalidName(_) => {
                return Err(SkillError::Zip("the skill name is not valid".to_owned()));
            }
            SkillIssue::UnsafePath(file) => {
                return Err(SkillError::Zip(format!(
                    "the file path {file:?} is not valid"
                )));
            }
            _ => {}
        }
    }
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let mut files: Vec<&SkillFileData> = pkg.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let skill_md = render_skill_md(pkg);
    let all = std::iter::once((SKILL_MD, skill_md.as_str())).chain(
        files
            .into_iter()
            .map(|f| (f.path.as_str(), f.content.as_str())),
    );
    for (file, content) in all {
        writer
            .start_file(format!("{}/{file}", pkg.name), options)
            .map_err(zip_error)?;
        writer.write_all(content.as_bytes()).map_err(io_error)?;
    }
    let bytes = writer.finish().map_err(zip_error)?.into_inner();
    fs::write(path, bytes).map_err(io_error)
}

//! `glob` and `grep` on Windows itself (AI-48, AI-49, §7.5), where `find` and `rg` cannot run:
//! the walk the hosts' scripts do with `find`, and rg's own regex engine on each file, printing
//! what `find` and `rg --null` print, so `hatoba_ai::search` reads the result as it reads a
//! host's. As there, links to files are searched and links to directories are not followed, `.git`
//! is skipped, binary files (a NUL byte) are left out, and the output stops at the bytes asked for.
//! Unlike there, `^` and `$` also take CRLF line breaks, which Windows files often have.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use hatoba_ssh::remote_search::{GrepMode, GrepQuery, RemoteSearchError, Scope, SearchOutput};
use regex::bytes::{Regex, RegexBuilder};

use super::files::{resolve, shown};

/// A file larger than this is not read whole for a multiline search.
const MAX_MULTILINE_FILE: u64 = 256 * 1024 * 1024;

fn line(value: &str, empty: bool) -> Result<&str, RemoteSearchError> {
    if (value.is_empty() && !empty) || value.contains(['\n', '\r', '\0']) {
        Err(RemoteSearchError::InvalidArgument)
    } else {
        Ok(value)
    }
}

/// Where the search starts, resolved as the scripts resolve it.
fn root(scope: &Scope<'_>, home: &Path) -> Result<PathBuf, RemoteSearchError> {
    line(scope.path, false)?;
    line(scope.below, true)?;
    for name in scope.names {
        line(name, false)?;
    }
    let mut path = resolve(scope.path, home);
    if !scope.below.is_empty() && path.is_dir() {
        // Joined as text and resolved again, so `/` and `c/Users` are Git Bash's `/c/Users`.
        let slash = if scope.path.ends_with(['/', '\\']) {
            ""
        } else {
            "/"
        };
        path = resolve(&format!("{}{slash}{}", scope.path, scope.below), home);
    }
    match fs::symlink_metadata(&path) {
        Ok(link) if link.file_type().is_symlink() && fs::metadata(&path).is_err() => {
            Err(RemoteSearchError::DanglingLink)
        }
        Ok(_) => Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(RemoteSearchError::PermissionDenied)
        }
        Err(_) => Err(RemoteSearchError::NotFound),
    }
}

/// What `find` would print for a directory: `dir/.`.
fn start_of(dir: &Path) -> String {
    let shown = shown(dir);
    if shown.ends_with('/') {
        format!("{shown}.")
    } else {
        format!("{shown}/.")
    }
}

/// Output that stops one byte past `max`, which tells that more followed.
struct Capped {
    out: Vec<u8>,
    max: usize,
}

impl Capped {
    fn new(max: u64) -> Self {
        Self {
            out: Vec::new(),
            max: usize::try_from(max).unwrap_or(usize::MAX),
        }
    }

    /// `false` once it is full.
    fn push(&mut self, bytes: &[u8]) -> bool {
        let room = self.max.saturating_add(1) - self.out.len();
        if bytes.len() >= room {
            self.out.extend_from_slice(&bytes[..room]);
            return false;
        }
        self.out.extend_from_slice(bytes);
        true
    }

    fn finish(mut self) -> (Vec<u8>, bool) {
        let cut = self.out.len() > self.max;
        self.out.truncate(self.max);
        (self.out, cut)
    }
}

/// The files and links to files below `dir`, at most `max_depth` levels down, whose names match
/// one of `names` (any name without one), as `find` lists them: each as the path printed (`start`
/// and its path below `dir`) and where it is. `false` from `visit` stops the walk.
fn walk(
    dir: &Path,
    start: &str,
    scope: &Scope<'_>,
    stop: &AtomicBool,
    stderr: &mut Vec<u8>,
    visit: &mut dyn FnMut(&str, &Path) -> bool,
) {
    let mut pending = vec![(dir.to_path_buf(), String::new(), 0u32)];
    while let Some((path, relative, depth)) = pending.pop() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) => {
                let message = if e.kind() == std::io::ErrorKind::PermissionDenied {
                    "Permission denied".to_owned()
                } else {
                    e.to_string()
                };
                stderr
                    .extend_from_slice(format!("find: '{}': {message}\n", shown(&path)).as_bytes());
                continue;
            }
        };
        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.contains(['\n', '\r']) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let below = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let is_file = if kind.is_symlink() {
                fs::metadata(entry.path()).is_ok_and(|m| m.is_file())
            } else {
                kind.is_file()
            };
            if kind.is_dir() {
                if name != ".git" && scope.max_depth.is_none_or(|d| depth + 1 < d) {
                    dirs.push((entry.path(), below, depth + 1));
                }
            } else if is_file
                && (scope.names.is_empty() || scope.names.iter().any(|p| fnmatch(p, &name)))
                && !visit(&format!("{start}/{below}"), &entry.path())
            {
                return;
            }
        }
        // Depth first, in the order the directory gave them, as `find` goes.
        pending.extend(dirs.into_iter().rev());
    }
}

/// `find -name`: `*`, `?`, `[…]` (with `!` or `^` to negate and ranges), and `\` before a
/// character that stands for itself.
pub fn fnmatch(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    matches_from(&p, &n)
}

fn matches_from(p: &[char], n: &[char]) -> bool {
    match p.first() {
        None => n.is_empty(),
        Some('*') => (0..=n.len()).any(|i| matches_from(&p[1..], &n[i..])),
        Some('?') => !n.is_empty() && matches_from(&p[1..], &n[1..]),
        Some('[') => match (class(&p[1..]), n.first()) {
            (Some((set, len)), Some(c)) => set(*c) && matches_from(&p[len + 1..], &n[1..]),
            (None, Some(c)) => *c == '[' && matches_from(&p[1..], &n[1..]),
            (_, None) => false,
        },
        Some('\\') if p.len() > 1 => n.first() == Some(&p[1]) && matches_from(&p[2..], &n[1..]),
        Some(c) => n.first() == Some(c) && matches_from(&p[1..], &n[1..]),
    }
}

/// A bracket expression after its `[`: what it matches, and how many characters it takes up to
/// and with its `]`; `None` when it has no `]`.
#[expect(clippy::type_complexity, reason = "a matcher and its length")]
fn class(p: &[char]) -> Option<(Box<dyn Fn(char) -> bool>, usize)> {
    let negated = matches!(p.first(), Some('!' | '^'));
    let mut i = usize::from(negated);
    let mut ranges = Vec::new();
    let mut first = true;
    while i < p.len() {
        let c = p[i];
        if c == ']' && !first {
            let set = move |x: char| ranges.iter().any(|&(lo, hi)| lo <= x && x <= hi) != negated;
            return Some((Box::new(set), i + 1));
        }
        first = false;
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            ranges.push((c, p[i + 2]));
            i += 3;
        } else {
            ranges.push((c, c));
            i += 1;
        }
    }
    None
}

/// AI-49: the files in `scope`, a path per line after `start`, at most `max` bytes.
pub fn glob(
    scope: &Scope<'_>,
    max: u64,
    home: &Path,
    stop: &AtomicBool,
) -> Result<SearchOutput, RemoteSearchError> {
    let dir = root(scope, home)?;
    if !dir.is_dir() {
        return Err(RemoteSearchError::NotDirectory(shown(&dir)));
    }
    let start = start_of(&dir);
    let mut out = Capped::new(max);
    let mut stderr = Vec::new();
    walk(&dir, &start, scope, stop, &mut stderr, &mut |printed, _| {
        out.push(format!("{printed}\n").as_bytes())
    });
    let (stdout, cut) = out.finish();
    Ok(SearchOutput {
        start,
        rg: false,
        nul: false,
        stdout,
        cut,
        stderr,
    })
}

/// AI-48: `query`'s matches, as `rg --null` prints them for its mode, at most `max` bytes.
pub fn grep(
    query: &GrepQuery<'_>,
    max: u64,
    home: &Path,
    stop: &AtomicBool,
) -> Result<SearchOutput, RemoteSearchError> {
    line(query.pattern, false)?;
    let regex = RegexBuilder::new(query.pattern)
        .case_insensitive(query.ignore_case)
        .multi_line(true)
        .crlf(true)
        .dot_matches_new_line(query.multiline)
        .build()
        .map_err(|e| RemoteSearchError::BadPattern {
            rg: true,
            message: e
                .to_string()
                .lines()
                .last()
                .unwrap_or_default()
                .trim()
                .to_owned(),
        })?;
    let target = root(&query.scope, home)?;
    let mut out = Capped::new(max);
    let mut stderr = Vec::new();
    let mut unread = Vec::new();
    let start;
    if target.is_dir() {
        start = start_of(&target);
        walk(
            &target,
            &start,
            &query.scope,
            stop,
            &mut stderr,
            &mut |printed, path| search_file(&regex, query, printed, path, &mut out, &mut unread),
        );
    } else {
        start = shown(&target);
        search_file(&regex, query, &start, &target, &mut out, &mut unread);
    }
    stderr.extend_from_slice(&unread);
    let (stdout, cut) = out.finish();
    Ok(SearchOutput {
        start,
        rg: true,
        nul: true,
        stdout,
        cut,
        stderr,
    })
}

/// Searches one file and prints what the mode asks for. `false` once the output is full.
fn search_file(
    regex: &Regex,
    query: &GrepQuery<'_>,
    printed: &str,
    path: &Path,
    out: &mut Capped,
    stderr: &mut Vec<u8>,
) -> bool {
    let lines = match matching_lines(regex, query.multiline, path) {
        Ok(Some(lines)) => lines,
        // A binary file, as rg leaves it out.
        Ok(None) => return true,
        Err(e) => {
            let message = if e.kind() == std::io::ErrorKind::PermissionDenied {
                "Permission denied".to_owned()
            } else {
                e.to_string()
            };
            stderr.extend_from_slice(format!("rg: {printed}: {message}\n").as_bytes());
            return true;
        }
    };
    let matched = lines.iter().filter(|l| l.matched).count();
    if matched == 0 {
        return true;
    }
    match query.mode {
        GrepMode::Files => out.push(format!("{printed}\n").as_bytes()),
        GrepMode::Count => out.push(format!("{printed}:{matched}\n").as_bytes()),
        GrepMode::Lines { before, after } => {
            let shown = with_context(&lines, before as usize, after as usize);
            for i in shown {
                let l = &lines[i];
                let mut row = Vec::with_capacity(printed.len() + l.text.len() + 16);
                row.extend_from_slice(printed.as_bytes());
                row.push(0);
                row.extend_from_slice(
                    format!("{}{}", i + 1, if l.matched { ':' } else { '-' }).as_bytes(),
                );
                row.extend_from_slice(&l.text);
                row.push(b'\n');
                if !out.push(&row) {
                    return false;
                }
            }
            true
        }
    }
}

struct Line {
    text: Vec<u8>,
    matched: bool,
}

/// The file's lines, without their line breaks, each with whether a match touches it; `None` for
/// a binary file.
fn matching_lines(
    regex: &Regex,
    multiline: bool,
    path: &Path,
) -> std::io::Result<Option<Vec<Line>>> {
    let file = File::open(path)?;
    if multiline {
        if file.metadata()?.len() > MAX_MULTILINE_FILE {
            return Ok(Some(Vec::new()));
        }
        let mut bytes = Vec::new();
        BufReader::new(file).read_to_end(&mut bytes)?;
        if bytes.contains(&0) {
            return Ok(None);
        }
        let mut starts = vec![0usize];
        starts.extend(
            bytes
                .iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i + 1),
        );
        if starts.last() == Some(&bytes.len()) && !bytes.is_empty() {
            starts.pop();
        }
        let line_of = |at: usize| starts.partition_point(|&s| s <= at).saturating_sub(1);
        let mut lines: Vec<Line> = starts
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let end = starts
                    .get(i + 1)
                    .map_or(bytes.len(), |&e| e.saturating_sub(1));
                Line {
                    text: bytes[s..end.max(s)].to_vec(),
                    matched: false,
                }
            })
            .collect();
        for m in regex.find_iter(&bytes) {
            let last = if m.end() > m.start() {
                m.end() - 1
            } else {
                m.start()
            };
            for line in line_of(m.start())..=line_of(last) {
                if let Some(l) = lines.get_mut(line) {
                    l.matched = true;
                }
            }
        }
        return Ok(Some(lines));
    }
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        if buf.contains(&0) {
            return Ok(None);
        }
        if buf.last() == Some(&b'\n') {
            buf.pop();
        }
        lines.push(Line {
            matched: regex.is_match(&buf),
            text: buf.clone(),
        });
    }
    Ok(Some(lines))
}

/// The lines a content result shows: each matching line and up to `before` and `after` lines
/// around it, in order, each once.
fn with_context(lines: &[Line], before: usize, after: usize) -> Vec<usize> {
    let mut shown = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if !l.matched {
            continue;
        }
        let from = i.saturating_sub(before);
        let to = (i + after).min(lines.len() - 1);
        for j in from..=to {
            if shown.last().is_none_or(|&last| j > last) {
                shown.push(j);
            }
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/deep")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join("src/main.rs"),
            "fn main() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        fs::write(root.join("src/deep/lib.rs"), "pub fn lib() {}\n").unwrap();
        fs::write(root.join("README.md"), "Hello\nworld\n").unwrap();
        fs::write(root.join(".git/config"), "fn hidden\n").unwrap();
        fs::write(root.join("bin.dat"), b"fn\0binary").unwrap();
        dir
    }

    fn scope<'a>(path: &'a str, names: &'a [String], max_depth: Option<u32>) -> Scope<'a> {
        Scope {
            path,
            below: "",
            max_depth,
            names,
        }
    }

    fn lines(out: &SearchOutput) -> Vec<String> {
        let mut l: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_owned)
            .collect();
        l.sort();
        l
    }

    #[test]
    fn fnmatch_reads_find_patterns() {
        assert!(fnmatch("*.rs", "main.rs"));
        assert!(!fnmatch("*.rs", "main.rsx"));
        assert!(fnmatch("ma?n.[rt]s", "main.rs"));
        assert!(fnmatch("[!a-c]*", "dog"));
        assert!(!fnmatch("[!a-c]*", "cat"));
        assert!(fnmatch("a\\*b", "a*b"));
        assert!(fnmatch("[]x]", "]"));
    }

    #[test]
    fn glob_walks_like_find() {
        let dir = tree();
        let home = dir.path();
        let stop = AtomicBool::new(false);
        let names = vec!["*.rs".to_owned()];
        let out = glob(&scope("~", &names, None), 1 << 20, home, &stop).unwrap();
        let start = start_of(home);
        assert_eq!(out.start, start);
        assert_eq!(
            lines(&out),
            [
                format!("{start}/src/deep/lib.rs"),
                format!("{start}/src/main.rs")
            ]
        );
        let out = glob(&scope("~", &[], Some(1)), 1 << 20, home, &stop).unwrap();
        assert_eq!(
            lines(&out),
            [format!("{start}/README.md"), format!("{start}/bin.dat")]
        );
        assert!(matches!(
            glob(&scope("README.md", &[], None), 100, home, &stop),
            Err(RemoteSearchError::NotDirectory(_))
        ));
        assert!(matches!(
            glob(&scope("none", &[], None), 100, home, &stop),
            Err(RemoteSearchError::NotFound)
        ));
        let out = glob(&scope("~", &[], None), 10, home, &stop).unwrap();
        assert!(out.cut && out.stdout.len() == 10);
    }

    #[cfg(windows)]
    #[test]
    fn a_glob_starts_below_a_drive_or_a_git_bash_path() {
        let dir = tree();
        let full = shown(dir.path());
        let (drive, rest) = full.split_at(2);
        let names = vec!["*.md".to_owned()];
        let stop = AtomicBool::new(false);
        let root_of = |path: &str, below: &str| {
            let scope = Scope {
                path,
                below,
                max_depth: Some(1),
                names: &names,
            };
            shown(&root(&scope, Path::new("~")).unwrap())
        };
        let drive_root = format!("{drive}/");
        assert_eq!(root_of(&drive_root, ""), drive_root);
        assert_eq!(root_of(&drive_root, &rest[1..]), full);
        // Git Bash's /c/Users/…: the glob's / and c/Users/….
        let letter = drive[..1].to_ascii_lowercase();
        assert_eq!(root_of("/", &format!("{letter}{rest}")), full);
        let scope = Scope {
            path: &drive_root,
            below: &rest[1..],
            max_depth: Some(1),
            names: &names,
        };
        let out = glob(&scope, 1 << 20, Path::new("~"), &stop).unwrap();
        assert_eq!(lines(&out), [format!("{full}/./README.md")]);
    }

    fn query<'a>(scope: Scope<'a>, pattern: &'a str, mode: GrepMode) -> GrepQuery<'a> {
        GrepQuery {
            scope,
            pattern,
            extended: None,
            ignore_case: false,
            multiline: false,
            mode,
        }
    }

    #[test]
    fn grep_prints_what_rg_null_prints() {
        let dir = tree();
        let home = dir.path();
        let stop = AtomicBool::new(false);
        let start = start_of(home);
        let out = grep(
            &query(scope("~", &[], None), "fn", GrepMode::Files),
            1 << 20,
            home,
            &stop,
        )
        .unwrap();
        assert_eq!(
            lines(&out),
            [
                format!("{start}/src/deep/lib.rs"),
                format!("{start}/src/main.rs")
            ]
        );
        let out = grep(
            &query(scope("~", &[], None), "l", GrepMode::Count),
            1 << 20,
            home,
            &stop,
        )
        .unwrap();
        assert_eq!(
            lines(&out),
            [
                format!("{start}/README.md:2"),
                format!("{start}/src/deep/lib.rs:1"),
                format!("{start}/src/main.rs:1")
            ]
        );
        let out = grep(
            &query(
                scope("src/main.rs", &[], None),
                "println",
                GrepMode::Lines {
                    before: 1,
                    after: 0,
                },
            ),
            1 << 20,
            home,
            &stop,
        )
        .unwrap();
        let file = shown(&home.join("src/main.rs"));
        assert_eq!(out.start, file);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{file}\x001-fn main() {{\n{file}\x002:    println!(\"hi\");\n")
        );
        let mut multi = query(scope("~", &[], None), "Hello.world", GrepMode::Files);
        multi.multiline = true;
        let out = grep(&multi, 1 << 20, home, &stop).unwrap();
        assert_eq!(lines(&out), [format!("{start}/README.md")]);
        let mut anchored = query(scope("~", &[], None), "^Hello$", GrepMode::Count);
        fs::write(home.join("crlf.txt"), "Hello\r\nworld\r\n").unwrap();
        let out = grep(&anchored, 1 << 20, home, &stop).unwrap();
        assert_eq!(
            lines(&out),
            [
                format!("{start}/README.md:1"),
                format!("{start}/crlf.txt:1")
            ]
        );
        anchored.multiline = true;
        anchored.pattern = r"^Hello$\s+^world$";
        let out = grep(&anchored, 1 << 20, home, &stop).unwrap();
        assert_eq!(
            lines(&out),
            [
                format!("{start}/README.md:2"),
                format!("{start}/crlf.txt:2")
            ]
        );
        assert!(matches!(
            grep(
                &query(scope("~", &[], None), "(", GrepMode::Files),
                100,
                home,
                &stop
            ),
            Err(RemoteSearchError::BadPattern { rg: true, .. })
        ));
    }
}

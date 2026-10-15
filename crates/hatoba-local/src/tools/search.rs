//! `glob` and `grep` on Windows itself (AI-48, AI-49, §7.5), where `find` and `rg` cannot run:
//! the walk the hosts' scripts do with `find`, and rg's own regex engine on each file, printing
//! what `find` and `rg --null` print, so `hatoba_ai::search` reads the result as it reads a
//! host's. As there, links to files are searched and links to directories are not followed, `.git`
//! is skipped, binary files (a NUL byte) are left out, and the output stops at the bytes asked for.
//! Unlike there, `^` and `$` also take CRLF line breaks, which Windows files often have. As with
//! rg, a multiline search reads a file whole only when the pattern can span lines, and then counts
//! matches rather than matching lines, which a match across lines would count more than once.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use hatoba_ssh::remote_search::{GrepMode, GrepQuery, RemoteSearchError, Scope, SearchOutput};
use regex::bytes::{Regex, RegexBuilder};
use regex_syntax::hir::{Class, Hir, HirKind, Look};

use super::files::{resolve, shown};

/// A multiline search reads a file whole, and reports a file larger than this as not searched.
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

    /// The bytes it takes before it is full.
    fn room(&self) -> usize {
        self.max.saturating_add(1) - self.out.len()
    }

    /// `false` once it is full.
    fn push(&mut self, bytes: &[u8]) -> bool {
        let room = self.room();
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
    let whole = query.multiline && spans_lines(query);
    let target = root(&query.scope, home)?;
    let mut out = Capped::new(max);
    let mut stderr = Vec::new();
    let mut unread = Vec::new();
    let mut search = |printed: &str, path: &Path| {
        search_file(
            &regex,
            whole,
            query.mode,
            printed,
            path,
            &mut out,
            &mut unread,
        )
    };
    let start;
    if target.is_dir() {
        start = start_of(&target);
        walk(
            &target,
            &start,
            &query.scope,
            stop,
            &mut stderr,
            &mut search,
        );
    } else {
        start = shown(&target);
        search(&start, &target);
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

/// Whether rg's multiline search reads files whole for `query`'s pattern, rather than line by
/// line: when a match can take in a line break, or the pattern has `^`, `$`, `\A` or `\z`, as rg's
/// `non_matching_bytes` tells.
fn spans_lines(query: &GrepQuery<'_>) -> bool {
    regex_syntax::ParserBuilder::new()
        .case_insensitive(query.ignore_case)
        .multi_line(true)
        .crlf(true)
        .dot_matches_new_line(true)
        .utf8(false)
        .build()
        .parse(query.pattern)
        .ok()
        .is_none_or(|hir| takes_line_break(&hir))
}

fn takes_line_break(hir: &Hir) -> bool {
    match hir.kind() {
        HirKind::Empty => false,
        HirKind::Literal(literal) => literal.0.contains(&b'\n'),
        HirKind::Class(Class::Unicode(class)) => class
            .ranges()
            .iter()
            .any(|r| r.start() <= '\n' && '\n' <= r.end()),
        HirKind::Class(Class::Bytes(class)) => class
            .ranges()
            .iter()
            .any(|r| r.start() <= b'\n' && b'\n' <= r.end()),
        HirKind::Look(look) => matches!(
            look,
            Look::Start | Look::End | Look::StartLF | Look::EndLF | Look::StartCRLF | Look::EndCRLF
        ),
        HirKind::Repetition(repetition) => takes_line_break(&repetition.sub),
        HirKind::Capture(capture) => takes_line_break(&capture.sub),
        HirKind::Concat(all) | HirKind::Alternation(all) => all.iter().any(takes_line_break),
    }
}

/// Searches one file, whole or line by line, and prints what the mode asks for. `false` once the
/// output is full.
fn search_file(
    regex: &Regex,
    whole: bool,
    mode: GrepMode,
    printed: &str,
    path: &Path,
    out: &mut Capped,
    stderr: &mut Vec<u8>,
) -> bool {
    let mut rows = Rows::new(mode, printed, out.room());
    let count = match search(regex, whole, path, &mut rows) {
        Ok(Some(count)) => count,
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
    if count == 0 {
        return true;
    }
    match mode {
        GrepMode::Files => out.push(format!("{printed}\n").as_bytes()),
        GrepMode::Count => out.push(format!("{printed}:{count}\n").as_bytes()),
        GrepMode::Lines { .. } => out.push(&rows.out),
    }
}

/// What a content result prints of one file, as `rg --null -n` prints it: each matching line and
/// up to `before` and `after` lines around it, in order, each once. It keeps only the lines that
/// may come before the next match, and stops a row past `room`, what the output still holds.
struct Rows<'a> {
    printed: &'a str,
    /// The lines of context before and after a match; `None` when the mode prints no lines.
    context: Option<(usize, usize)>,
    /// The last lines not printed, at most `before`: each with its number.
    waiting: VecDeque<(usize, Vec<u8>)>,
    /// The lines of context still to print after the last match.
    after_left: usize,
    out: Vec<u8>,
    room: usize,
}

impl<'a> Rows<'a> {
    fn new(mode: GrepMode, printed: &'a str, room: usize) -> Self {
        let context = match mode {
            GrepMode::Lines { before, after } => Some((before as usize, after as usize)),
            GrepMode::Files | GrepMode::Count => None,
        };
        Self {
            printed,
            context,
            waiting: VecDeque::new(),
            after_left: 0,
            out: Vec::new(),
            room,
        }
    }

    /// Line `number` (from 1), without its line break.
    fn line(&mut self, number: usize, text: &[u8], matched: bool) {
        let Some((before, after)) = self.context else {
            return;
        };
        if matched {
            while let Some((n, t)) = self.waiting.pop_front() {
                self.row(n, &t, false);
            }
            self.row(number, text, true);
            self.after_left = after;
        } else if self.after_left > 0 {
            self.row(number, text, false);
            self.after_left -= 1;
        } else if before > 0 {
            let mut kept = if self.waiting.len() == before {
                self.waiting.pop_front().map(|(_, t)| t).unwrap_or_default()
            } else {
                Vec::new()
            };
            kept.clear();
            kept.extend_from_slice(text);
            self.waiting.push_back((number, kept));
        }
    }

    fn row(&mut self, number: usize, text: &[u8], matched: bool) {
        if self.out.len() > self.room {
            return;
        }
        self.out.extend_from_slice(self.printed.as_bytes());
        self.out.push(0);
        self.out
            .extend_from_slice(format!("{number}{}", if matched { ':' } else { '-' }).as_bytes());
        self.out.extend_from_slice(text);
        self.out.push(b'\n');
    }
}

/// Reads the file, whole or a line at a time, and hands `rows` each line with whether a match
/// touches it. What `rg -c` counts: the matches when it was read whole, else the matching lines;
/// `None` for a binary file.
fn search(
    regex: &Regex,
    whole: bool,
    path: &Path,
    rows: &mut Rows<'_>,
) -> std::io::Result<Option<usize>> {
    let file = File::open(path)?;
    if whole {
        if file.metadata()?.len() > MAX_MULTILINE_FILE {
            return Err(std::io::Error::other(format!(
                "not searched: larger than the {} MiB a multiline search reads",
                MAX_MULTILINE_FILE / 1024 / 1024
            )));
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
        let mut matched = vec![false; starts.len()];
        let mut count = 0;
        for m in regex.find_iter(&bytes) {
            count += 1;
            let last = if m.end() > m.start() {
                m.end() - 1
            } else {
                m.start()
            };
            for line in line_of(m.start())..=line_of(last) {
                if let Some(l) = matched.get_mut(line) {
                    *l = true;
                }
            }
        }
        for (i, &s) in starts.iter().enumerate() {
            let end = starts
                .get(i + 1)
                .map_or(bytes.len(), |&e| e.saturating_sub(1));
            rows.line(i + 1, &bytes[s..end.max(s)], matched[i]);
        }
        return Ok(Some(count));
    }
    let mut reader = BufReader::new(file);
    let mut count = 0;
    let mut number = 0;
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
        number += 1;
        let matched = regex.is_match(&buf);
        count += usize::from(matched);
        rows.line(number, &buf, matched);
    }
    Ok(Some(count))
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
                format!("{start}/README.md:1"),
                format!("{start}/crlf.txt:1")
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

    #[test]
    fn content_shows_each_line_of_context_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::write(home.join("c.txt"), "1\nx2\n3\n4\n5\nx6\n7\nx8\n9\n10\n").unwrap();
        let file = shown(&home.join("c.txt"));
        let stop = AtomicBool::new(false);
        let rows = |pattern: &str, multiline: bool, before: u32, after: u32| {
            let mut q = query(
                scope("c.txt", &[], None),
                pattern,
                GrepMode::Lines { before, after },
            );
            q.multiline = multiline;
            let out = grep(&q, 1 << 20, home, &stop).unwrap();
            String::from_utf8(out.stdout)
                .unwrap()
                .lines()
                .map(|l| l.strip_prefix(&format!("{file}\0")).unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        let expected = ["1-1", "2:x2", "3-3", "5-5", "6:x6", "7-7", "8:x8", "9-9"];
        assert_eq!(rows("x", false, 1, 1), expected);
        // Read whole, with the same rows.
        assert_eq!(rows("^x", true, 1, 1), expected);
        assert_eq!(rows("x", false, 0, 0), ["2:x2", "6:x6", "8:x8"]);
        assert_eq!(rows("x8", false, 3, 0), ["5-5", "6-x6", "7-7", "8:x8"]);
        assert_eq!(rows(r"x6\n7", true, 0, 1), ["6:x6", "7:7", "8-x8"]);
        // The output stops at the bytes asked for.
        let q = query(
            scope("c.txt", &[], None),
            ".",
            GrepMode::Lines {
                before: 0,
                after: 0,
            },
        );
        let out = grep(&q, 10, home, &stop).unwrap();
        assert!(out.cut && out.stdout.len() == 10);
    }

    #[test]
    fn a_multiline_count_counts_what_rg_c_u_counts() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::write(home.join("u.txt"), "ab ab\nab\n").unwrap();
        let file = shown(&home.join("u.txt"));
        let stop = AtomicBool::new(false);
        let count = |pattern: &str, multiline: bool| {
            let mut q = query(scope("u.txt", &[], None), pattern, GrepMode::Count);
            q.multiline = multiline;
            String::from_utf8(grep(&q, 1 << 20, home, &stop).unwrap().stdout).unwrap()
        };
        // What rg 14 prints: the matches where the pattern can span lines, else the matching
        // lines, as without multiline.
        for (pattern, n) in [
            (r"ab", 2),
            (r"\ba", 2),
            (r"a(?-s:.)", 2),
            (r"a.", 3),
            (r"\s", 3),
            (r"\Ab|a", 3),
            (r"b$|a", 5),
        ] {
            assert_eq!(count(pattern, true), format!("{file}:{n}\n"), "{pattern}");
        }
        assert_eq!(count(r"b$|a", false), format!("{file}:2\n"));
        // A match across two lines is one.
        fs::write(home.join("u.txt"), "Hello\nworld\n").unwrap();
        assert_eq!(count(r"Hello.world", true), format!("{file}:1\n"));
    }

    #[test]
    fn a_multiline_search_reports_a_file_too_large_to_read_whole() {
        let dir = tree();
        let home = dir.path();
        File::create(home.join("big.log"))
            .unwrap()
            .set_len(MAX_MULTILINE_FILE + 1)
            .unwrap();
        let stop = AtomicBool::new(false);
        let start = start_of(home);
        let mut q = query(scope("~", &[], None), "Hello.world", GrepMode::Files);
        q.multiline = true;
        let out = grep(&q, 1 << 20, home, &stop).unwrap();
        assert_eq!(lines(&out), [format!("{start}/README.md")]);
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            format!(
                "rg: {start}/big.log: not searched: larger than the 256 MiB a multiline search reads\n"
            )
        );
        q.scope.path = "big.log";
        let out = grep(&q, 1 << 20, home, &stop).unwrap();
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("big.log: not searched"));
    }
}

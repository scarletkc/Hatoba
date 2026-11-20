use std::collections::{BTreeMap, BTreeSet};

use super::glob::is_drive;
use super::pattern::counts_matches;
use super::{Search, context};
use crate::tools::{GrepArgs, GrepOutput, MAX_RESULT_CHARS};

/// Paths a `glob` result lists at most.
pub const MAX_GLOB_PATHS: usize = 1_000;
/// Entries a `grep` result lists unless `limit` says otherwise.
pub const DEFAULT_GREP_LIMIT: u64 = 250;
/// Characters of a line a `grep` result shows; the rest is cut.
pub const MAX_LINE_CHARS: usize = 500;
/// Bytes of `find`, `rg` and `grep` output read from the host; a search stops there.
pub const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
/// Distinct error lines of `find`, `rg` and `grep` a result quotes.
const MAX_ERRORS: usize = 3;

/// What `find`, `rg` and `grep` printed on the host, as the desktop shell read it.
#[derive(Clone, Copy, Debug)]
pub struct Found<'a> {
    /// Where `find` started, as the script resolved it: a directory with `/.` after it, or a
    /// file.
    pub start: &'a str,
    /// A path per line for `glob`, the lines of `rg` or `grep` for `grep`.
    pub stdout: &'a [u8],
    /// Each line of a file's content follows the file and a NUL, rather than `:` or `-`.
    pub nul: bool,
    /// More output followed [`MAX_OUTPUT_BYTES`]; it was not read.
    pub cut: bool,
    pub stderr: &'a [u8],
}

/// Where the paths `find` printed start.
pub(super) struct Root {
    /// The directory or file searched, as the result names it.
    shown: String,
    /// What each path below the directory searched starts with; `None` for a file, which `find`
    /// prints as it is.
    prefix: Option<String>,
}

impl Root {
    pub(super) fn of(start: &str) -> Self {
        match start.strip_suffix("/.") {
            Some(dir) => Self {
                shown: if dir.is_empty() {
                    "/".to_owned()
                } else if is_drive(dir) {
                    format!("{dir}/")
                } else {
                    dir.to_owned()
                },
                prefix: Some(format!("{start}/")),
            },
            None => Self {
                shown: start.to_owned(),
                prefix: None,
            },
        }
    }

    /// A path `find` printed, as the result shows it, if the search keeps it.
    pub(super) fn kept(&self, printed: &str, search: &Search) -> Option<String> {
        match &self.prefix {
            Some(prefix) => {
                let relative = printed.strip_prefix(prefix.as_str())?;
                search.matches(relative).then(|| {
                    if self.shown.ends_with('/') {
                        format!("{}{relative}", self.shown)
                    } else {
                        format!("{}/{relative}", self.shown)
                    }
                })
            }
            // A file given as the path is searched whatever the glob says, as rg searches a file
            // it is given.
            None => (printed == self.shown).then(|| self.shown.clone()),
        }
    }

    /// Where the search looked, for the result's first line.
    pub(super) fn place(&self, search: &Search) -> String {
        match self.prefix {
            Some(_) => format!("under {}", search.base(&self.shown)),
            None => format!("in {}", self.shown),
        }
    }
}

/// The output's lines, without a last one that the cap cut short.
fn raw_lines<'a>(found: &Found<'a>) -> Vec<&'a [u8]> {
    let mut lines: Vec<&[u8]> = found.stdout.split(|&b| b == b'\n').collect();
    if found.cut || lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// The output's lines, without any with a NUL byte, which a grep that cannot skip binary files
/// prints.
fn lines(found: &Found<'_>) -> Vec<String> {
    raw_lines(found)
        .into_iter()
        .filter(|l| !l.contains(&0))
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect()
}

/// What `find`, `rg` and `grep` reported: how many files or directories they could not read, and
/// the other errors worth naming.
#[derive(Default)]
pub(super) struct Problems {
    pub(super) denied: usize,
    pub(super) other: Vec<String>,
}

pub(super) fn problems(stderr: &[u8]) -> Problems {
    let mut problems = Problems::default();
    // `find` and `rg` may both report a directory.
    let mut denied = BTreeSet::new();
    let text = String::from_utf8_lossy(stderr);
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        // `rg` adds the error's number.
        let message = match line.rsplit_once(" (os error ") {
            Some((message, number)) if number.ends_with(')') => message,
            _ => line,
        };
        if let Some(rest) = message.strip_suffix(": Permission denied") {
            let path = rest.split_once(": ").map_or(rest, |(_, path)| path);
            denied.insert(path.trim_matches(['\'', '‘', '’', '"']));
        } else if !passed_over(message)
            && problems.other.len() < MAX_ERRORS
            && !problems.other.iter().any(|o| o == line)
        {
            problems.other.push(line.to_owned());
        }
    }
    problems.denied = denied.len();
    problems
}

/// Errors about files gone, or replaced by directories, while the search ran, and links to
/// nothing, which a result leaves out.
fn passed_over(message: &str) -> bool {
    message.ends_with("Is a directory") || message.ends_with("No such file or directory")
}

/// The notes after a result: output past the cap, and what `find`, `rg` and `grep` reported.
fn notes(out: &mut String, found: &Found<'_>, tool: &str) {
    if found.cut {
        out.push_str(&format!(
            "[Hatoba stopped reading after {} MB of results, so some are missing. Narrow the \
             path, the glob or the pattern.]\n",
            MAX_OUTPUT_BYTES / 1024 / 1024
        ));
    }
    let problems = problems(found.stderr);
    if problems.denied > 0 {
        out.push_str(&format!(
            "[{} could not be read: permission denied. {tool} never uses sudo; to search them, \
             use send_input to run the search with sudo in the terminal's shell.]\n",
            count(problems.denied, "file or directory", "files or directories")
        ));
    }
    for line in &problems.other {
        out.push_str(&format!("[The host reported: {line}]\n"));
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// AI-49: the `glob` result: the matching paths, sorted, at most [`MAX_GLOB_PATHS`] and the size
/// of a result. `pattern` is the glob as the model gave it.
#[must_use]
pub fn glob_result(pattern: &str, search: &Search, found: &Found<'_>) -> String {
    let root = Root::of(found.start);
    let mut paths: Vec<String> = lines(found)
        .iter()
        .filter_map(|line| root.kept(line, search))
        .collect();
    paths.sort();
    paths.dedup();
    let place = root.place(search);
    let pattern = pattern.trim();
    let mut out = if paths.is_empty() {
        format!("No files {place} match {pattern}.\n")
    } else {
        let verb = if paths.len() == 1 { "matches" } else { "match" };
        format!(
            "{} {place} {verb} {pattern}, sorted by path:\n",
            count(paths.len(), "file", "files")
        )
    };
    let budget = MAX_RESULT_CHARS - 1_000;
    let mut used = 0;
    let mut shown = 0;
    for path in paths.iter().take(MAX_GLOB_PATHS) {
        let size = path.chars().count() + 1;
        if used + size > budget {
            break;
        }
        out.push_str(path);
        out.push('\n');
        used += size;
        shown += 1;
    }
    if shown < paths.len() {
        out.push_str(&format!(
            "[{} more are not shown. Narrow the pattern or the path to see them.]\n",
            paths.len() - shown
        ));
    }
    notes(&mut out, found, "glob");
    out
}

/// One entry of a `grep` result: a file, a count, or a line; or the `--` between groups of
/// lines, which is shown but not counted.
struct Entry {
    text: String,
    counted: bool,
}

/// A file's lines in a content result.
struct Hits {
    path: String,
    rows: Vec<Row>,
}

enum Row {
    Line {
        number: u64,
        matched: bool,
        text: String,
    },
    /// The lines around one match and the next are apart.
    Gap,
}

/// A line after its file: `N:text` for a matching line, or `N-text` for a line of context.
fn numbered(rest: &str) -> Option<Row> {
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let matched = match rest[digits..].chars().next() {
        Some(':') => true,
        Some('-') => false,
        _ => return None,
    };
    (digits > 0).then(|| Row::Line {
        number: rest[..digits].parse().unwrap_or_default(),
        matched,
        text: rest[digits + 1..].to_owned(),
    })
}

/// A line `grep -n` printed for the file `printed` without `--null`: `printed:N:text` for a
/// matching line, or `printed-N-text` for a line of context.
fn row_of(line: &str, printed: &str) -> Option<Row> {
    let rest = line.strip_prefix(printed)?;
    let separator = rest.chars().next().filter(|&c| c == ':' || c == '-')?;
    let row = numbered(&rest[1..])?;
    matches!(row, Row::Line { matched, .. } if matched == (separator == ':')).then_some(row)
}

/// A matching line of a file below `root` without `--null`: the file as printed, and the line.
/// The file is what comes before the first `:N:`, so a name with `:N:` in it is read wrong.
fn matching_line<'a>(line: &'a str, root: &Root) -> Option<(&'a str, Row)> {
    let start = root.prefix.as_deref().unwrap_or(&root.shown);
    let from = line.starts_with(start).then_some(start.len())?;
    line[from..].match_indices(':').find_map(|(at, _)| {
        let printed = &line[..from + at];
        match row_of(line, printed) {
            Some(row @ Row::Line { matched: true, .. }) => Some((printed, row)),
            _ => None,
        }
    })
}

/// The files and lines of `grep -n` output without `--null`, in its order. The lines of context
/// before a group's first matching line are read once that line names the file; where two runs of
/// grep meet, a line of a file named like the one before plus `-N-` can be read as that file's.
fn content(lines: &[String], root: &Root) -> Vec<Hits> {
    let mut files: Vec<Hits> = Vec::new();
    let mut current: Option<String> = None;
    let mut before: Vec<&str> = Vec::new();
    let mut gap = false;
    for line in lines {
        if line == "--" {
            current = None;
            before.clear();
            gap = true;
            continue;
        }
        if let Some(printed) = &current
            && let Some(row) = row_of(line, printed)
            && let Some(file) = files.last_mut()
        {
            file.rows.push(row);
            continue;
        }
        let Some((printed, row)) = matching_line(line, root) else {
            // Context before a match, or after one of another file than `current`.
            current = None;
            before.push(line);
            continue;
        };
        match files.last_mut() {
            Some(file) if file.path == printed => {
                if gap {
                    file.rows.push(Row::Gap);
                }
            }
            _ => files.push(Hits {
                path: printed.to_owned(),
                rows: Vec::new(),
            }),
        }
        let file = files.last_mut().expect("pushed above");
        file.rows
            .extend(before.drain(..).filter_map(|l| row_of(l, printed)));
        file.rows.push(row);
        current = Some(printed.to_owned());
        gap = false;
    }
    files
}

/// The files and lines of output where each line follows its file and a NUL, as `rg --null` and
/// `grep --null` print it: by file, with a gap where the line numbers skip.
fn content_nul(found: &Found<'_>) -> Vec<Hits> {
    let mut files: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for line in raw_lines(found) {
        let Some(at) = line.iter().position(|&b| b == 0) else {
            // `--`, or what `rg` says of a binary file.
            continue;
        };
        let (path, rest) = (&line[..at], &line[at + 1..]);
        if rest.contains(&0) {
            continue;
        }
        let Some(row) = numbered(&String::from_utf8_lossy(rest)) else {
            continue;
        };
        let rows = files
            .entry(String::from_utf8_lossy(path).into_owned())
            .or_default();
        if let (Some(Row::Line { number: last, .. }), Row::Line { number, .. }) =
            (rows.last(), &row)
            && *number != last + 1
        {
            rows.push(Row::Gap);
        }
        rows.push(row);
    }
    files
        .into_iter()
        .map(|(path, rows)| Hits { path, rows })
        .collect()
}

/// A line's text as a result shows it, cut to [`MAX_LINE_CHARS`]; the flag says it was cut.
fn shown_text(text: &str) -> (String, bool) {
    let text = text.strip_suffix('\r').unwrap_or(text);
    match text.char_indices().nth(MAX_LINE_CHARS) {
        Some((end, _)) => (format!("{} [… cut]", &text[..end]), true),
        None => (text.to_owned(), false),
    }
}

/// The counted entries from `offset` on, at most `limit` and what fits `budget` characters, with
/// the separators between them: the text, and how many were shown.
fn page(entries: &[Entry], offset: usize, limit: usize, budget: usize) -> (String, usize) {
    let mut body = String::new();
    let (mut used, mut skipped, mut shown, mut separate) = (0, 0, 0, false);
    for entry in entries {
        if !entry.counted {
            separate = shown > 0;
            continue;
        }
        if skipped < offset {
            skipped += 1;
            continue;
        }
        let size = entry.text.chars().count() + 1 + if separate { 3 } else { 0 };
        if shown == limit || (shown > 0 && used + size > budget) {
            break;
        }
        if separate {
            body.push_str("--\n");
        }
        body.push_str(&entry.text);
        body.push('\n');
        used += size;
        shown += 1;
        separate = false;
    }
    (body, shown)
}

/// AI-48: the `grep` result for `args` from what `rg` or `grep` printed: the files, the counts or
/// the lines, sorted by path, a page of them as `limit` and `offset` ask, with how to read on.
#[must_use]
pub fn grep_result(args: &GrepArgs, search: &Search, found: &Found<'_>) -> String {
    let root = Root::of(found.start);
    let mode = args.output_mode.unwrap_or_default();
    let place = root.place(search);
    let mut cut = false;
    // The entries, the summary of what matched, and what an entry is.
    let (entries, summary, kind) = match mode {
        GrepOutput::FilesWithMatches => {
            let mut paths: Vec<String> = lines(found)
                .iter()
                .filter_map(|line| root.kept(line, search))
                .collect();
            paths.sort();
            paths.dedup();
            let summary = format!(
                "{} with a matching line {place}",
                count(paths.len(), "file", "files")
            );
            let entries = paths
                .into_iter()
                .map(|text| Entry {
                    text,
                    counted: true,
                })
                .collect::<Vec<_>>();
            (entries, summary, "files")
        }
        GrepOutput::Count => {
            let mut counts: Vec<(String, u64)> = lines(found)
                .iter()
                .filter_map(|line| {
                    let (printed, n) = line.rsplit_once(':')?;
                    Some((root.kept(printed, search)?, n.parse().ok()?))
                })
                .collect();
            counts.sort();
            let total =
                usize::try_from(counts.iter().map(|(_, n)| n).sum::<u64>()).unwrap_or(usize::MAX);
            let (one, many) = if counts_matches(args) {
                ("match", "matches")
            } else {
                ("matching line", "matching lines")
            };
            let summary = format!(
                "{} in {} {place}",
                count(total, one, many),
                count(counts.len(), "file", "files")
            );
            let entries = counts
                .into_iter()
                .map(|(path, n)| Entry {
                    text: format!("{path}:{n}"),
                    counted: true,
                })
                .collect::<Vec<_>>();
            (entries, summary, "files")
        }
        GrepOutput::Content => {
            let hits = if found.nul {
                content_nul(found)
            } else {
                content(&lines(found), &root)
            };
            let mut files: Vec<Hits> = hits
                .into_iter()
                .filter_map(|hits| {
                    Some(Hits {
                        path: root.kept(&hits.path, search)?,
                        rows: hits.rows,
                    })
                })
                .collect();
            files.sort_by(|a, b| a.path.cmp(&b.path));
            let matched = files
                .iter()
                .flat_map(|f| &f.rows)
                .filter(|r| matches!(r, Row::Line { matched: true, .. }))
                .count();
            let summary = format!(
                "{} in {} {place}",
                count(matched, "matching line", "matching lines"),
                count(files.len(), "file", "files")
            );
            let context = context(args) != (0, 0);
            let numbers = args.line_numbers.unwrap_or(true);
            let mut entries = Vec::new();
            for (i, file) in files.iter().enumerate() {
                for (j, row) in file.rows.iter().enumerate() {
                    match row {
                        Row::Gap => {}
                        Row::Line {
                            number,
                            matched,
                            text,
                        } => {
                            let after_gap = j
                                .checked_sub(1)
                                .is_some_and(|k| matches!(file.rows[k], Row::Gap));
                            let apart = after_gap || (j == 0 && i > 0);
                            if context && apart {
                                entries.push(Entry {
                                    text: String::new(),
                                    counted: false,
                                });
                            }
                            let (text, was_cut) = shown_text(text);
                            cut |= was_cut;
                            let separator = if *matched { ':' } else { '-' };
                            let text = if numbers {
                                format!("{}{separator}{number}{separator}{text}", file.path)
                            } else {
                                format!("{}{separator}{text}", file.path)
                            };
                            entries.push(Entry {
                                text,
                                counted: true,
                            });
                        }
                    }
                }
            }
            (entries, summary, "lines")
        }
    };
    let total = entries.iter().filter(|e| e.counted).count();
    let offset = usize::try_from(args.offset.unwrap_or(0)).unwrap_or(usize::MAX);
    let limit = match args.limit.unwrap_or(DEFAULT_GREP_LIMIT) {
        0 => usize::MAX,
        limit => usize::try_from(limit).unwrap_or(usize::MAX),
    };
    let mut out = String::new();
    if total == 0 {
        out.push_str(&format!("No lines match the pattern {place}.\n"));
    } else if offset >= total {
        out.push_str(&format!(
            "{summary}. offset {offset} is past the last of them, so none are shown.\n"
        ));
    } else {
        let (body, shown) = page(&entries, offset, limit, MAX_RESULT_CHARS - 1_000);
        let (first, last) = (offset + 1, offset + shown);
        if first == 1 && last == total {
            out.push_str(&format!("{summary}, sorted by path:\n"));
        } else {
            out.push_str(&format!(
                "{summary}, sorted by path; {kind} {first}–{last}:\n"
            ));
        }
        out.push_str(&body);
        if last < total {
            out.push_str(&format!(
                "[{} {}–{total} are not shown. To see them, call grep again with offset \
                 {last}.]\n",
                if kind == "files" { "Files" } else { "Lines" },
                last + 1
            ));
        }
        if cut {
            out.push_str(&format!(
                "[Lines longer than {MAX_LINE_CHARS} characters are cut. read_file shows more of \
                 them.]\n"
            ));
        }
    }
    notes(&mut out, found, "grep");
    out
}

//! The text side of the search tools (spec §13.4, AI-48, AI-49): the glob that `glob` matches and
//! `grep` picks files by, the regular expression `grep` hands to the host's `grep -E`, and the
//! results, built from what `find` and `grep` printed. Running them on the host is the desktop
//! shell's part, with the scripts in `hatoba_ssh::remote_search`.

use crate::tools::{GrepArgs, GrepOutput, MAX_RESULT_CHARS};

/// Paths a `glob` result lists at most.
pub const MAX_GLOB_PATHS: usize = 1_000;
/// Entries a `grep` result lists unless `limit` says otherwise.
pub const DEFAULT_GREP_LIMIT: u64 = 250;
/// Lines of context `grep` shows at most before and after a matching line.
pub const MAX_CONTEXT: u64 = 100;
/// Characters of a line a `grep` result shows; the rest is cut.
pub const MAX_LINE_CHARS: usize = 500;
/// Bytes of `find` and `grep` output read from the host; a search stops there.
pub const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
/// Alternatives the braces of a glob may expand to.
const MAX_ALTERNATIVES: usize = 64;
/// Distinct error lines of `find` and `grep` a result quotes.
const MAX_ERRORS: usize = 3;

/// Where a search starts and which files it looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    /// The path the host searches: the path the model gave, the home directory (`~`) without one,
    /// or `/` for a glob that starts with `/`, followed by the directories the glob starts with.
    pub path: String,
    /// `find -maxdepth`, when no alternative of the glob has `**`.
    pub max_depth: Option<u32>,
    /// `find -name` patterns, one of which a file's name matches; empty for any name. With
    /// `max_depth` they narrow what the host prints to a superset of what [`Search::matches`]
    /// keeps.
    pub names: Vec<String>,
    /// The directories of the glob that `path` ends with, as `a/b`; empty for none.
    moved: String,
    /// The glob's alternatives, each a path below `path` by component; empty without a glob.
    globs: Vec<Vec<Part>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    /// `**`: any number of directories.
    Dirs,
    /// One name, matched as `fnmatch` matches it.
    Name(String),
}

impl Search {
    /// The search of `path` (the home directory when `None` or empty) for the files `glob`
    /// matches, or all of them without one. A glob without `/` matches file names at any depth;
    /// one with `/` matches paths below `path`, and one that starts with `/` paths below `/`.
    /// An error is the message for the model.
    pub fn new(path: Option<&str>, glob: Option<&str>) -> Result<Self, String> {
        let mut path = path.filter(|p| !p.is_empty()).unwrap_or("~").to_owned();
        let none = Self {
            path: String::new(),
            max_depth: None,
            names: Vec::new(),
            moved: String::new(),
            globs: Vec::new(),
        };
        let Some(given) = glob.map(str::trim).filter(|g| !g.is_empty()) else {
            return Ok(Self { path, ..none });
        };
        if given.contains(['\n', '\r', '\0']) {
            return Err("The glob has a line break, so nothing was searched.".to_owned());
        }
        let absolute = given.starts_with('/');
        if absolute {
            path = "/".to_owned();
        }
        let mut rest = given.trim_start_matches('/');
        while let Some(after) = rest.strip_prefix("./") {
            rest = after.trim_start_matches('/');
        }
        let mut globs = Vec::new();
        for alternative in expand_braces(rest.trim_end_matches('/'))? {
            let mut parts = Vec::new();
            if !absolute && !alternative.contains('/') {
                parts.push(Part::Dirs);
            }
            for name in alternative
                .split('/')
                .filter(|n| !n.is_empty() && *n != ".")
            {
                if name != "**" {
                    parts.push(Part::Name(name.to_owned()));
                } else if parts.last() != Some(&Part::Dirs) {
                    parts.push(Part::Dirs);
                }
            }
            if !parts.is_empty() && !globs.contains(&parts) {
                globs.push(parts);
            }
        }
        if globs.is_empty() {
            return Err(format!(
                "The glob {given} names no files. Give a pattern such as *.conf or **/*.conf."
            ));
        }
        // A glob that starts with directories searches them, rather than all of `path`.
        let mut moved = Vec::new();
        if let [parts] = globs.as_mut_slice() {
            while parts.len() > 1
                && let Part::Name(name) = &parts[0]
                && !name.contains(['*', '?', '[', '\\'])
            {
                moved.push(name.clone());
                parts.remove(0);
            }
        }
        for name in &moved {
            if !path.ends_with('/') {
                path.push('/');
            }
            path.push_str(name);
        }
        let max_depth = globs
            .iter()
            .all(|parts| !parts.contains(&Part::Dirs))
            .then(|| globs.iter().map(Vec::len).max().unwrap_or(1))
            .map(|depth| u32::try_from(depth).unwrap_or(u32::MAX));
        let mut names = Vec::new();
        for parts in &globs {
            match parts.last() {
                Some(Part::Name(name)) => {
                    let name = find_name(name);
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                _ => {
                    names.clear();
                    break;
                }
            }
        }
        Ok(Self {
            path,
            max_depth,
            names,
            moved: moved.join("/"),
            globs,
        })
    }

    /// The search of the `glob` tool: as [`Search::new`], but an empty `pattern` is refused
    /// rather than matching every file.
    pub fn glob(path: Option<&str>, pattern: &str) -> Result<Self, String> {
        if pattern.trim().is_empty() {
            return Err(
                "The pattern is empty, so nothing was searched. Give one such as *.conf or \
                 **/*.conf."
                    .to_owned(),
            );
        }
        Self::new(path, Some(pattern))
    }

    /// Whether the glob matches a file at `relative`, its path below [`Search::path`]; every file
    /// matches without a glob.
    #[must_use]
    pub fn matches(&self, relative: &str) -> bool {
        if self.globs.is_empty() {
            return true;
        }
        let names: Vec<&str> = relative.split('/').collect();
        self.globs.iter().any(|parts| matches_parts(parts, &names))
    }

    /// The directory the model asked to search, from `root`, the directory searched as the host
    /// resolved it: without the directories the glob started with.
    fn base(&self, root: &str) -> String {
        if self.moved.is_empty() {
            return root.to_owned();
        }
        match root.strip_suffix(&format!("/{}", self.moved)) {
            Some("") => "/".to_owned(),
            Some(base) => base.to_owned(),
            None => root.to_owned(),
        }
    }
}

/// The alternatives of a glob's braces, `{a,b}`, nested or not; braces without a comma, or
/// without a closing brace, stand for themselves.
fn expand_braces(glob: &str) -> Result<Vec<String>, String> {
    fn expand(glob: &str, out: &mut Vec<String>) -> Result<(), String> {
        let Some((open, close, commas)) = first_group(glob) else {
            if out.len() == MAX_ALTERNATIVES {
                return Err(format!(
                    "The glob's braces give more than {MAX_ALTERNATIVES} alternatives, so nothing \
                     was searched. Use fewer, or a broader pattern."
                ));
            }
            out.push(glob.to_owned());
            return Ok(());
        };
        let (head, tail) = (&glob[..open], &glob[close + 1..]);
        let mut start = open + 1;
        for end in commas.into_iter().chain([close]) {
            expand(&format!("{head}{}{tail}", &glob[start..end]), out)?;
            start = end + 1;
        }
        Ok(())
    }
    let mut out = Vec::new();
    expand(glob, &mut out)?;
    Ok(out)
}

/// The first `{…}` with a comma at its own level: where it opens and closes, and its commas.
fn first_group(glob: &str) -> Option<(usize, usize, Vec<usize>)> {
    let bytes = glob.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b'{' => {
                let (mut depth, mut commas, mut j) = (0, Vec::new(), i);
                while j < bytes.len() {
                    match bytes[j] {
                        b'\\' => j += 1,
                        b'{' => depth += 1,
                        b',' if depth == 1 => commas.push(j),
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if j < bytes.len() && !commas.is_empty() {
                    return Some((i, j, commas));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A name of a glob as `find -name` reads it everywhere: `[^…]` as `[!…]`.
fn find_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        match c {
            '\\' => out.extend(chars.next()),
            '[' if chars.peek() == Some(&'^') => {
                chars.next();
                out.push('!');
            }
            _ => {}
        }
    }
    out
}

fn matches_parts(parts: &[Part], names: &[&str]) -> bool {
    match parts.split_first() {
        None => names.is_empty(),
        // A glob that ends with `**` matches what is inside, not the directory itself.
        Some((Part::Dirs, [])) => !names.is_empty(),
        Some((Part::Dirs, rest)) => (0..=names.len()).any(|i| matches_parts(rest, &names[i..])),
        Some((Part::Name(pattern), rest)) => names
            .split_first()
            .is_some_and(|(name, more)| fnmatch(pattern, name) && matches_parts(rest, more)),
    }
}

/// Whether `name` matches `pattern` as `fnmatch` without flags matches it: `*`, `?`, `[…]` with
/// `!` or `^` to negate, ranges and `[:class:]`, and `\` to take the next character as it is. A
/// leading dot needs no dot in the pattern, as with `find -name`.
fn fnmatch(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = name.chars().collect();
    let (mut pi, mut si) = (0, 0);
    let mut star = None;
    while si < s.len() {
        if p.get(pi) == Some(&'*') {
            star = Some((pi, si));
            pi += 1;
        } else if pi < p.len()
            && let Some(next) = step(&p, pi, s[si])
        {
            pi = next;
            si += 1;
        } else if let Some((at, from)) = star {
            pi = at + 1;
            si = from + 1;
            star = Some((at, from + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Where `p` goes on after its token at `pi` matched `c`; `None` when it does not match.
fn step(p: &[char], pi: usize, c: char) -> Option<usize> {
    match p[pi] {
        '?' => Some(pi + 1),
        '\\' if pi + 1 < p.len() => (p[pi + 1] == c).then_some(pi + 2),
        '[' => match bracket(p, pi + 1, c) {
            Some((found, end)) => found.then_some(end),
            // Without a closing `]`, the `[` stands for itself.
            None => (c == '[').then_some(pi + 1),
        },
        other => (other == c).then_some(pi + 1),
    }
}

/// The bracket expression that starts at `i`, after its `[`: whether `c` is in it, and where the
/// pattern goes on after its `]`. `None` without a `]`.
fn bracket(p: &[char], mut i: usize, c: char) -> Option<(bool, usize)> {
    let negated = matches!(p.get(i), Some('!' | '^'));
    if negated {
        i += 1;
    }
    let mut found = false;
    let start = i;
    loop {
        let ch = *p.get(i)?;
        if ch == ']' && i > start {
            return Some((found != negated, i + 1));
        }
        if ch == '['
            && p.get(i + 1) == Some(&':')
            && let Some(len) = p[i + 2..].windows(2).position(|w| w == [':', ']'])
        {
            let class: String = p[i + 2..i + 2 + len].iter().collect();
            found |= in_class(&class, c);
            i += len + 4;
            continue;
        }
        let (low, next) = if ch == '\\' && i + 1 < p.len() {
            (p[i + 1], i + 2)
        } else {
            (ch, i + 1)
        };
        if p.get(next) == Some(&'-') && p.get(next + 1).is_some_and(|&h| h != ']') {
            let (high, after) = if p[next + 1] == '\\' && next + 2 < p.len() {
                (p[next + 2], next + 3)
            } else {
                (p[next + 1], next + 2)
            };
            found |= low <= c && c <= high;
            i = after;
        } else {
            found |= low == c;
            i = next;
        }
    }
}

fn in_class(class: &str, c: char) -> bool {
    match class {
        "alpha" => c.is_alphabetic(),
        "digit" => c.is_ascii_digit(),
        "alnum" => c.is_alphanumeric(),
        "upper" => c.is_uppercase(),
        "lower" => c.is_lowercase(),
        "space" => c.is_whitespace(),
        "punct" => c.is_ascii_punctuation(),
        "xdigit" => c.is_ascii_hexdigit(),
        _ => false,
    }
}

/// AI-48: `pattern` as every host's `grep -E` reads it: `\d`, `\s` and `\w`, which only some
/// greps know, and their negations become POSIX classes, and `\t` a tab, also inside a bracket
/// expression. An empty pattern, or one with a line break, which grep would take for several, is
/// refused with the message for the model.
pub fn extended_regex(pattern: &str) -> Result<String, String> {
    if pattern.is_empty() {
        return Err("The pattern is empty, so nothing was searched.".to_owned());
    }
    if pattern.contains(['\n', '\r', '\0']) {
        return Err(
            "The pattern has a line break, and grep matches one line at a time, so nothing was \
             searched. Search for one of its lines."
                .to_owned(),
        );
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut in_bracket = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if in_bracket {
            // `[:class:]`, `[.x.]` and `[=x=]` go through whole.
            if c == '['
                && let Some(delimiter @ (':' | '.' | '=')) = next
                && let Some(end) =
                    (i + 2..chars.len() - 1).find(|&j| chars[j] == delimiter && chars[j + 1] == ']')
            {
                out.extend(&chars[i..end + 2]);
                i = end + 2;
                continue;
            }
            match (c, next.and_then(bracket_shorthand)) {
                ('\\', Some(class)) => {
                    out.push_str(class);
                    i += 2;
                }
                (']', _) => {
                    in_bracket = false;
                    out.push(c);
                    i += 1;
                }
                _ => {
                    out.push(c);
                    i += 1;
                }
            }
            continue;
        }
        match c {
            '\\' => {
                match next {
                    Some(n) => match shorthand(n) {
                        Some(class) => out.push_str(class),
                        None => {
                            out.push('\\');
                            out.push(n);
                        }
                    },
                    None => out.push('\\'),
                }
                i += 2;
            }
            '[' => {
                in_bracket = true;
                out.push('[');
                i += 1;
                if chars.get(i) == Some(&'^') {
                    out.push('^');
                    i += 1;
                }
                // A `]` first in the expression stands for itself.
                if chars.get(i) == Some(&']') {
                    out.push(']');
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

fn shorthand(c: char) -> Option<&'static str> {
    Some(match c {
        'd' => "[0-9]",
        'D' => "[^0-9]",
        's' => "[[:space:]]",
        'S' => "[^[:space:]]",
        'w' => "[[:alnum:]_]",
        'W' => "[^[:alnum:]_]",
        't' => "\t",
        _ => return None,
    })
}

fn bracket_shorthand(c: char) -> Option<&'static str> {
    Some(match c {
        'd' => "0-9",
        's' => "[:space:]",
        'w' => "[:alnum:]_",
        't' => "\t",
        _ => return None,
    })
}

/// What `find` and `grep` printed on the host, as the desktop shell read it.
#[derive(Clone, Copy, Debug)]
pub struct Found<'a> {
    /// Where `find` started, as the script resolved it: a directory with `/.` after it, or a
    /// file.
    pub start: &'a str,
    /// A path per line for `glob`, the lines of `grep` for `grep`.
    pub stdout: &'a [u8],
    /// More output followed [`MAX_OUTPUT_BYTES`]; it was not read.
    pub cut: bool,
    pub stderr: &'a [u8],
}

/// Where the paths `find` printed start.
struct Root {
    /// The directory or file searched, as the result names it.
    shown: String,
    /// What each path below the directory searched starts with; `None` for a file, which `find`
    /// prints as it is.
    prefix: Option<String>,
}

impl Root {
    fn of(start: &str) -> Self {
        match start.strip_suffix("/.") {
            Some(dir) => Self {
                shown: if dir.is_empty() { "/" } else { dir }.to_owned(),
                prefix: Some(format!("{start}/")),
            },
            None => Self {
                shown: start.to_owned(),
                prefix: None,
            },
        }
    }

    /// A path `find` printed, as the result shows it, if the search keeps it.
    fn kept(&self, printed: &str, search: &Search) -> Option<String> {
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
    fn place(&self, search: &Search) -> String {
        match self.prefix {
            Some(_) => format!("under {}", search.base(&self.shown)),
            None => format!("in {}", self.shown),
        }
    }
}

/// The output's lines, without a last one that the cap cut short or any with a NUL byte, which
/// a grep that cannot skip binary files prints.
fn lines(found: &Found<'_>) -> Vec<String> {
    let mut lines: Vec<&[u8]> = found.stdout.split(|&b| b == b'\n').collect();
    if found.cut || lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
        .into_iter()
        .filter(|l| !l.contains(&0))
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect()
}

/// What `find` and `grep` reported: how many files or directories they could not read, and the
/// other errors worth naming.
#[derive(Default)]
struct Problems {
    denied: usize,
    other: Vec<String>,
}

fn problems(stderr: &[u8]) -> Problems {
    let mut problems = Problems::default();
    let text = String::from_utf8_lossy(stderr);
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.ends_with("Permission denied") {
            problems.denied += 1;
        } else if !passed_over(line)
            && problems.other.len() < MAX_ERRORS
            && !problems.other.iter().any(|o| o == line)
        {
            problems.other.push(line.to_owned());
        }
    }
    problems
}

/// Errors about files gone, or replaced by directories, while the search ran, which a result
/// leaves out.
fn passed_over(line: &str) -> bool {
    line.ends_with("Is a directory") || line.ends_with("No such file or directory")
}

/// The notes after a result: output past the cap, and what `find` and `grep` reported.
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

/// A line `grep -n` printed for the file `printed`: `printed:N:text` for a matching line, or
/// `printed-N-text` for a line of context.
fn row_of(line: &str, printed: &str) -> Option<Row> {
    let rest = line.strip_prefix(printed)?;
    let separator = rest.chars().next().filter(|&c| c == ':' || c == '-')?;
    let rest = &rest[1..];
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    if digits == 0 || !rest[digits..].starts_with(separator) {
        return None;
    }
    Some(Row::Line {
        number: rest[..digits].parse().ok()?,
        matched: separator == ':',
        text: rest[digits + 1..].to_owned(),
    })
}

/// A matching line of a file below `root`: the file as printed, and the line. The file is what
/// comes before the first `:N:`, so a name with `:N:` in it is read wrong; `grep` has no way to
/// mark where a name ends that every host knows.
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

/// The files and lines of `grep -n` output, in its order. The lines of context before a group's
/// first matching line are read once that line names the file.
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

/// AI-48: the `grep` result for `args` from what `find` and `grep` printed: the files, the counts
/// or the lines, sorted by path, a page of them as `limit` and `offset` ask, with how to read on.
#[must_use]
pub fn grep_result(args: &GrepArgs, search: &Search, found: &Found<'_>) -> String {
    let root = Root::of(found.start);
    let lines = lines(found);
    let mode = args.output_mode.unwrap_or_default();
    let place = root.place(search);
    let mut cut = false;
    // The entries, the summary of what matched, and what an entry is.
    let (entries, summary, kind) = match mode {
        GrepOutput::FilesWithMatches => {
            let mut paths: Vec<String> = lines
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
            let mut counts: Vec<(String, u64)> = lines
                .iter()
                .filter_map(|line| {
                    let (printed, n) = line.rsplit_once(':')?;
                    Some((root.kept(printed, search)?, n.parse().ok()?))
                })
                .collect();
            counts.sort();
            let total: u64 = counts.iter().map(|(_, n)| n).sum();
            let summary = format!(
                "{} in {} {place}",
                count(
                    usize::try_from(total).unwrap_or(usize::MAX),
                    "matching line",
                    "matching lines"
                ),
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
            let mut files: Vec<Hits> = content(&lines, &root)
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
            let context = args.context.unwrap_or(0) > 0;
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
                            entries.push(Entry {
                                text: format!("{}{separator}{number}{separator}{text}", file.path),
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
    let limit =
        usize::try_from(args.limit.unwrap_or(DEFAULT_GREP_LIMIT).max(1)).unwrap_or(usize::MAX);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn search(path: Option<&str>, glob: &str) -> Search {
        Search::new(path, Some(glob)).unwrap()
    }

    #[test]
    fn globs_match_names_at_any_depth_and_paths_from_the_start() {
        // No `/`: a name at any depth, as with rg's --glob and find -name.
        let s = search(Some("/etc/nginx"), "*.conf");
        assert_eq!(
            (s.path.as_str(), s.max_depth, &s.names[..]),
            ("/etc/nginx", None, &["*.conf".to_owned()][..])
        );
        for yes in ["nginx.conf", "conf.d/app.conf", "a/b/.hidden.conf"] {
            assert!(s.matches(yes), "{yes}");
        }
        assert!(!s.matches("nginx.conf.bak") && !s.matches("conf.d"));

        // With `/`: a path from where the search starts; * stays within a name.
        let s = search(None, "*/sites-*/*");
        assert_eq!(
            (s.path.as_str(), s.max_depth, &s.names[..]),
            ("~", Some(3), &["*".to_owned()][..])
        );
        assert!(s.matches("nginx/sites-enabled/api"));
        assert!(!s.matches("nginx/sites-enabled/old/api") && !s.matches("sites-enabled/api"));

        // ** is any number of directories, none included, and at the end only what is inside.
        let s = search(Some("/srv"), "app/**/*.env");
        assert_eq!((s.path.as_str(), s.max_depth), ("/srv/app", None));
        for yes in [".env", "x.env", "config/prod/x.env"] {
            assert!(s.matches(yes), "{yes}");
        }
        let s = search(Some("/srv"), "logs/**");
        assert!(s.matches("a") && s.matches("a/b") && s.names.is_empty());
    }

    #[test]
    fn globs_start_where_they_name_directories() {
        // The directories a glob starts with are searched directly, and the result names the
        // directory the model gave.
        let s = search(Some("/etc/"), "nginx/sites-enabled/*");
        assert_eq!(
            (s.path.as_str(), s.max_depth),
            ("/etc/nginx/sites-enabled", Some(1))
        );
        assert!(s.matches("api") && !s.matches("a/b"));
        assert_eq!(s.base("/etc/nginx/sites-enabled"), "/etc");
        // An absolute glob searches from /, whatever the path.
        let s = search(Some("/home/ops"), "/etc/nginx/**/*.conf");
        assert_eq!((s.path.as_str(), s.max_depth), ("/etc/nginx", None));
        assert!(s.matches("nginx.conf") && s.matches("conf.d/a.conf"));
        assert_eq!(s.base("/etc/nginx"), "/");
        let s = search(None, "/*.conf");
        assert_eq!((s.path.as_str(), s.max_depth), ("/", Some(1)));
        assert!(s.matches("x.conf") && !s.matches("etc/x.conf"));
        // `./` and a trailing `/` change nothing; a directory that is a pattern stays in it.
        assert_eq!(search(None, "./a/b/").path, "~/a");
        assert_eq!(search(Some("/"), "{etc,opt}/*.conf").path, "/");
        // A path the base cannot be found in is shown as resolved.
        assert_eq!(search(None, "a/b").base("/elsewhere"), "/elsewhere");
    }

    #[test]
    fn braces_classes_and_escapes() {
        let s = search(None, "*.{yml,yaml}");
        assert_eq!(s.names, ["*.yml", "*.yaml"]);
        assert!(s.matches("ci/a.yaml") && !s.matches("a.yml.bak"));
        let s = search(None, "{nginx/*.conf,apache2/**/*.conf}");
        assert_eq!((s.max_depth, s.names.len()), (None, 1));
        assert!(s.matches("nginx/a.conf") && s.matches("apache2/x/y.conf"));
        assert!(!s.matches("nginx/x/a.conf"));
        // Nested braces, braces without a comma, and an unclosed one.
        assert_eq!(
            expand_braces("a{b,c{d,e}}f").unwrap(),
            ["abf", "acdf", "acef"]
        );
        assert_eq!(expand_braces("{x}.{y").unwrap(), ["{x}.{y"]);
        assert!(expand_braces(&"{a,b}".repeat(7)).is_err());

        assert!(fnmatch("[a-c]?.log", "b1.log") && !fnmatch("[a-c]?.log", "d1.log"));
        assert!(fnmatch("[!.]*", "x") && !fnmatch("[^.]*", ".x"));
        assert!(fnmatch("[[:digit:]]*", "9x") && fnmatch("[]x]", "]"));
        assert!(fnmatch(r"\*.txt", "*.txt") && !fnmatch(r"\*.txt", "a.txt"));
        assert!(fnmatch("[", "[") && fnmatch("*", ".bashrc") && fnmatch("a*b*c", "aXbYbc"));
        assert!(!fnmatch("a*b", "acb/") && fnmatch("日*", "日本"));
        // find reads [! everywhere; [^ only some find do.
        assert_eq!(find_name("[^.]*"), "[!.]*");
        assert_eq!(find_name(r"\[^x]"), r"\[^x]");

        assert!(Search::new(None, Some("a\nb")).is_err());
        assert!(Search::new(None, Some("/")).is_err());
        // grep without a glob searches every file; glob needs a pattern.
        assert_eq!(Search::new(Some(""), Some("  ")).unwrap().path, "~");
        assert!(
            Search::glob(None, " ")
                .unwrap_err()
                .starts_with("The pattern is empty")
        );
        assert_eq!(Search::glob(Some("/etc"), "*.conf").unwrap().path, "/etc");
    }

    #[test]
    fn patterns_read_alike_on_every_grep() {
        let ere = |p: &str| extended_regex(p).unwrap();
        assert_eq!(ere(r"listen\s+\d+;"), "listen[[:space:]]+[0-9]+;");
        assert_eq!(
            ere(r"\w+\W\S\D"),
            "[[:alnum:]_]+[^[:alnum:]_][^[:space:]][^0-9]"
        );
        assert_eq!(ere(r"[\d.]+:[\w-]"), "[0-9.]+:[[:alnum:]_-]");
        assert_eq!(ere(r"a\tb"), "a\tb");
        // Other escapes, an escaped backslash and POSIX classes stay as written.
        assert_eq!(ere(r"\.\bx\\d"), r"\.\bx\\d");
        assert_eq!(ere(r"[[:digit:]\d]"), "[[:digit:]0-9]");
        assert_eq!(ere(r"[]\d]"), "[]0-9]");
        assert_eq!(ere(r"[^]x]\d"), "[^]x][0-9]");
        assert_eq!(ere(r"end\"), r"end\");
        assert_eq!(ere("日本\\s"), "日本[[:space:]]");
        assert!(extended_regex("").is_err());
        assert!(extended_regex("a\nb").is_err());
    }

    fn found<'a>(start: &'a str, stdout: &'a str, stderr: &'a str) -> Found<'a> {
        Found {
            start,
            stdout: stdout.as_bytes(),
            cut: false,
            stderr: stderr.as_bytes(),
        }
    }

    fn grep(json: &str) -> GrepArgs {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn glob_results_are_sorted_and_filtered() {
        let s = search(Some("/etc"), "nginx/sites-*/*");
        let out = glob_result(
            "nginx/sites-*/*",
            &s,
            &found(
                "/etc/nginx/.",
                "/etc/nginx/./sites-enabled/b\n/etc/nginx/./sites-enabled/a\n/etc/nginx/./nginx.\
                 conf\n",
                "find: '/etc/nginx/private': Permission denied\n",
            ),
        );
        assert_eq!(
            out,
            "2 files under /etc match nginx/sites-*/*, sorted by path:\n/etc/nginx/sites-enabled/\
             a\n/etc/nginx/sites-enabled/b\n[1 file or directory could not be read: permission \
             denied. glob never uses sudo; to search them, use send_input to run the search with \
             sudo in the terminal's shell.]\n"
        );
        let none = glob_result("*.rs", &search(None, "*.rs"), &found("/home/ops/.", "", ""));
        assert_eq!(none, "No files under /home/ops match *.rs.\n");
        // From /, with a capped output whose last line was cut short.
        let mut cut = found("/.", "/./etc/a.conf\n/./etc/b.co", "");
        cut.cut = true;
        let out = glob_result("*.conf", &search(Some("/"), "*.conf"), &cut);
        assert!(out.starts_with("1 file under / matches *.conf, sorted by path:\n/etc/a.conf\n"));
        assert!(out.contains("stopped reading after 4 MB"));

        let many: String = (0..1_200).map(|i| format!("/x/./f{i:04}\n")).collect();
        let out = glob_result("*", &search(Some("/x"), "*"), &found("/x/.", &many, ""));
        assert!(out.contains("/x/f0999\n[200 more are not shown."), "{out}");
    }

    #[test]
    fn grep_lists_files_and_counts() {
        let s = search(Some("/etc"), "*.conf");
        let args = grep(r#"{"pattern":"8080"}"#);
        let out = grep_result(
            &args,
            &s,
            &found(
                "/etc/.",
                "/etc/./b.conf\n/etc/./a.conf\n/etc/./a.conf.bak\n",
                "grep: /etc/./c.conf: Permission denied\ngrep: /etc/./link: Is a directory\n\
                 grep: warning: stray \\ before d\n",
            ),
        );
        assert_eq!(
            out,
            "2 files with a matching line under /etc, sorted by path:\n/etc/a.conf\n/etc/b.conf\n\
             [1 file or directory could not be read: permission denied. grep never uses sudo; to \
             search them, use send_input to run the search with sudo in the terminal's shell.]\n\
             [The host reported: grep: warning: stray \\ before d]\n"
        );
        let args = grep(r#"{"pattern":"x","output_mode":"count"}"#);
        let out = grep_result(
            &args,
            &search(Some("/srv"), ""),
            &found("/srv/.", "/srv/./b:3\n/srv/./a:x:1\n", ""),
        );
        assert_eq!(
            out,
            "4 matching lines in 2 files under /srv, sorted by path:\n/srv/a:x:1\n/srv/b:3\n"
        );
        // A file searched on its own, whatever the glob says, as with rg.
        let out = grep_result(
            &grep(r#"{"pattern":"x","output_mode":"count","glob":"*.conf"}"#),
            &search(Some("/etc/hosts"), "*.conf"),
            &found("/etc/hosts", "/etc/hosts:2\n", ""),
        );
        assert!(out.starts_with("2 matching lines in 1 file in /etc/hosts"));
        let none = grep_result(&args, &search(None, ""), &found("/root/.", "", ""));
        assert_eq!(none, "No lines match the pattern under /root.\n");
    }

    #[test]
    fn grep_shows_lines_with_their_context() {
        let s = Search::new(Some("/etc/nginx"), None).unwrap();
        let stdout = "/etc/nginx/./sites/b-1-server {\n/etc/nginx/./sites/b:2:  listen 8080;\n\
                      /etc/nginx/./sites/b-3-}\n--\n/etc/nginx/./sites/b-9-x\n/etc/nginx/./sites/b:\
                      10:  listen 8080;\n/etc/nginx/./a-2:3:x\n/etc/nginx/./a-2:4:8080\n";
        let out = grep_result(
            &grep(r#"{"pattern":"8080","output_mode":"content","context":1}"#),
            &s,
            &found("/etc/nginx/.", stdout, ""),
        );
        assert_eq!(
            out,
            "4 matching lines in 2 files under /etc/nginx, sorted by path:\n\
             /etc/nginx/a-2:3:x\n/etc/nginx/a-2:4:8080\n--\n/etc/nginx/sites/b-1-server {\n\
             /etc/nginx/sites/b:2:  listen 8080;\n/etc/nginx/sites/b-3-}\n--\n\
             /etc/nginx/sites/b-9-x\n/etc/nginx/sites/b:10:  listen 8080;\n"
        );
        // Pages count lines, not separators, and say where to read on.
        let out = grep_result(
            &grep(r#"{"pattern":"8080","output_mode":"content","context":1,"limit":3,"offset":2}"#),
            &s,
            &found("/etc/nginx/.", stdout, ""),
        );
        assert_eq!(
            out,
            "4 matching lines in 2 files under /etc/nginx, sorted by path; lines 3–5:\n\
             /etc/nginx/sites/b-1-server {\n/etc/nginx/sites/b:2:  listen 8080;\n\
             /etc/nginx/sites/b-3-}\n[Lines 6–7 are not shown. To see them, call grep again \
             with offset 5.]\n"
        );
        let past = grep_result(
            &grep(r#"{"pattern":"8080","output_mode":"content","offset":9}"#),
            &s,
            &found("/etc/nginx/.", stdout, ""),
        );
        assert!(past.contains("offset 9 is past the last of them"));
    }

    #[test]
    fn grep_lines_from_several_runs_and_long_lines() {
        // Two runs of grep in a row (find -exec … +) print no -- between them; a long line and
        // a CRLF line end are cut.
        let long = "y".repeat(MAX_LINE_CHARS + 10);
        let stdout = format!(
            "/s/./a:1:x\r\n/s/./a-2-after\n/s/./b-4-before\n/s/./b:5:{long}\n/s/./c\0:1:bin\n"
        );
        let out = grep_result(
            &grep(r#"{"pattern":"x","output_mode":"content","context":1,"glob":"[ab]"}"#),
            &search(Some("/s"), "[ab]"),
            &found("/s/.", &stdout, ""),
        );
        assert_eq!(
            out,
            format!(
                "2 matching lines in 2 files under /s, sorted by path:\n/s/a:1:x\n/s/a-2-after\n\
                 --\n/s/b-4-before\n/s/b:5:{} [… cut]\n[Lines longer than 500 characters are cut. \
                 read_file shows more of them.]\n",
                "y".repeat(MAX_LINE_CHARS)
            )
        );
        // Without context, a file's lines follow each other.
        let out = grep_result(
            &grep(r#"{"pattern":"x","output_mode":"content"}"#),
            &search(Some("/s"), "*"),
            &found("/s/.", "/s/./a:1:x\n/s/./a:3:x\n/s/./b:2:x\n", ""),
        );
        assert!(out.ends_with(":\n/s/a:1:x\n/s/a:3:x\n/s/b:2:x\n"), "{out}");
        // A file searched on its own, with lines that do not start with its path.
        let out = grep_result(
            &grep(r#"{"pattern":"x","output_mode":"content"}"#),
            &Search::new(Some("/etc/hosts"), None).unwrap(),
            &found(
                "/etc/hosts",
                "x\nBinary file /etc/hosts matches\n/etc/hosts:3:x\n",
                "",
            ),
        );
        assert_eq!(
            out,
            "1 matching line in 1 file in /etc/hosts, sorted by path:\n/etc/hosts:3:x\n"
        );
    }
}

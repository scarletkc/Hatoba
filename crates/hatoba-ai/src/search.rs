//! The text side of the search tools (spec §13.4, AI-48, AI-49): the glob that `glob` matches and
//! `grep` picks files by, the pattern `grep` hands to the host's `rg`, or to its `grep -E` as
//! translated here, and the results, built from what `find`, `rg` and `grep` printed. Running them
//! on the host is the desktop shell's part, with the scripts in `hatoba_ssh::remote_search`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use regex_syntax::ast::{self, Ast};
use regex_syntax::hir::{Class, Hir, HirKind, Look};

use crate::tools::{GrepArgs, GrepOutput, MAX_RESULT_CHARS};

/// Paths a `glob` result lists at most.
pub const MAX_GLOB_PATHS: usize = 1_000;
/// Entries a `grep` result lists unless `limit` says otherwise.
pub const DEFAULT_GREP_LIMIT: u64 = 250;
/// Lines of context `grep` shows at most before and after a matching line.
pub const MAX_CONTEXT: u64 = 100;
/// Characters of a line a `grep` result shows; the rest is cut.
pub const MAX_LINE_CHARS: usize = 500;
/// Bytes of `find`, `rg` and `grep` output read from the host; a search stops there.
pub const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
/// Alternatives the braces of a glob may expand to.
const MAX_ALTERNATIVES: usize = 64;
/// Distinct error lines of `find`, `rg` and `grep` a result quotes.
const MAX_ERRORS: usize = 3;

/// Where a search starts and which files it looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    /// The path the model gave, the home directory (`~`) without one, `/` for a glob that starts
    /// with `/`, or a drive's root, such as `C:/`, for one that starts with it (§7.5).
    pub path: String,
    /// The directories the glob starts with, as `a/b`, which the host searches below `path` when
    /// that is a directory; empty for none.
    pub below: String,
    /// `find -maxdepth`, when no alternative of the glob has `**`.
    pub max_depth: Option<u32>,
    /// Name patterns, one of which a file's name matches; empty for any name. Each means the same
    /// to `find -name` and `rg --glob`, or more. With `max_depth` they narrow what the host prints
    /// to a superset of what [`Search::matches`] keeps.
    pub names: Vec<String>,
    /// The glob's alternatives, each a path below `path` and `below` by component; empty without a
    /// glob.
    globs: Vec<Vec<Part>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    /// `**`: any number of directories.
    Dirs,
    /// One name, matched as `fnmatch` matches it.
    Name(Vec<char>),
}

impl Search {
    /// The search of `path` (the home directory when `None` or empty) for the files `glob`
    /// matches, or all of them without one. A glob without `/` matches file names at any depth;
    /// one with `/` matches paths below `path`, and one that starts with `/`, or with a drive as
    /// `C:/` does, paths below it. An error is the message for the model.
    pub fn new(path: Option<&str>, glob: Option<&str>) -> Result<Self, String> {
        let path = path.filter(|p| !p.is_empty()).unwrap_or("~").to_owned();
        let none = Self {
            path: String::new(),
            below: String::new(),
            max_depth: None,
            names: Vec::new(),
            globs: Vec::new(),
        };
        let Some(given) = glob.map(str::trim).filter(|g| !g.is_empty()) else {
            return Ok(Self { path, ..none });
        };
        if given.contains(['\n', '\r', '\0']) {
            return Err("The glob has a line break, so nothing was searched.".to_owned());
        }
        let drive = drive_root(given);
        let absolute = given.starts_with('/') || drive.is_some();
        let rest = if drive.is_some() { &given[2..] } else { given };
        let path = match drive {
            Some(root) => root,
            None if absolute => "/".to_owned(),
            None => path,
        };
        let mut rest = rest.trim_start_matches('/');
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
                    parts.push(Part::Name(name.chars().collect()));
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
        let mut below = Vec::new();
        if let [parts] = globs.as_mut_slice() {
            while parts.len() > 1
                && let Part::Name(name) = &parts[0]
                && !name.iter().any(|c| matches!(c, '*' | '?' | '[' | '\\'))
            {
                below.push(name.iter().collect::<String>());
                parts.remove(0);
            }
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
                    let name = host_name(name);
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
            below: below.join("/"),
            max_depth,
            names,
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

    /// The path with the directories below it that the host searches when `path` is a
    /// directory, for messages.
    #[must_use]
    pub fn shown(&self) -> String {
        if self.below.is_empty() {
            self.path.clone()
        } else if self.path.ends_with('/') {
            format!("{}{}", self.path, self.below)
        } else {
            format!("{}/{}", self.path, self.below)
        }
    }

    /// Whether the glob matches a file at `relative`, its path below the directory searched; every
    /// file matches without a glob.
    #[must_use]
    pub fn matches(&self, relative: &str) -> bool {
        self.globs.is_empty()
            || self
                .globs
                .iter()
                .any(|parts| matches_parts(parts, relative.split('/')))
    }

    /// The directory the model asked to search, from `root`, the directory searched as the host
    /// resolved it: without the directories the glob started with.
    fn base(&self, root: &str) -> String {
        if self.below.is_empty() {
            return root.to_owned();
        }
        match root
            .strip_suffix(self.below.as_str())
            .and_then(|r| r.strip_suffix('/'))
        {
            Some("") => "/".to_owned(),
            Some(base) if is_drive(base) => format!("{base}/"),
            Some(base) => base.to_owned(),
            None => root.to_owned(),
        }
    }
}

/// The root of the drive a glob starts with, such as `C:/` for `C:/Projects/*.json`.
fn drive_root(glob: &str) -> Option<String> {
    let bytes = glob.as_bytes();
    (bytes.len() > 2 && bytes[2] == b'/' && is_drive(&glob[..2]))
        .then(|| format!("{}:/", char::from(bytes[0]).to_ascii_uppercase()))
}

/// `C:`: a drive, without its root.
fn is_drive(text: &str) -> bool {
    matches!(text.as_bytes(), [letter, b':'] if letter.is_ascii_alphabetic())
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

/// A name of a glob as both `find -name` and `rg --glob` read it: braces, and a `!` or `#` that
/// starts it or spaces that end it, which `rg` reads as its own syntax, escaped; `[^…]` as `[!…]`,
/// which every `find` reads; a `[` without `]` escaped, and `**` as `*`, which `rg` refuses; and a
/// bracket expression with a class such as `[:digit:]`, which `rg` does not know, as `?`, any
/// character, which [`Search::matches`] then narrows.
fn host_name(name: &[char]) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let trailing = name.len() - name.iter().rev().take_while(|&&c| c == ' ').count();
    let mut i = 0;
    while i < name.len() {
        let c = name[i];
        match c {
            '\\' if i + 1 < name.len() => {
                out.push('\\');
                out.push(name[i + 1]);
                i += 2;
                continue;
            }
            '\\' => out.push_str("\\\\"),
            '[' => match bracket_end(name, i) {
                Some(end) if name[i..end].windows(2).any(|w| w == ['[', ':']) && i + 1 < end => {
                    out.push('?');
                    i = end;
                    continue;
                }
                Some(end) => {
                    out.push('[');
                    let mut j = i + 1;
                    if name[j] == '^' {
                        out.push('!');
                        j += 1;
                    }
                    out.extend(&name[j..end]);
                    i = end;
                    continue;
                }
                None => out.push_str("\\["),
            },
            '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '!' | '#' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            ' ' if i >= trailing => out.push_str("\\ "),
            '*' if out.ends_with('*') && !out.ends_with("\\*") => {}
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// Where the bracket expression that opens at `name[open]` ends, after its `]`; `None` without
/// one.
fn bracket_end(name: &[char], open: usize) -> Option<usize> {
    let mut i = open + 1;
    if matches!(name.get(i), Some('!' | '^')) {
        i += 1;
    }
    let start = i;
    while let Some(&c) = name.get(i) {
        match c {
            ']' if i > start => return Some(i + 1),
            '[' if name.get(i + 1) == Some(&':') => {
                let len = name[i + 2..].windows(2).position(|w| w == [':', ']'])?;
                i += len + 4;
                continue;
            }
            '\\' => i += 1,
            _ => {}
        }
        i += 1;
    }
    None
}

fn matches_parts(parts: &[Part], mut names: std::str::Split<'_, char>) -> bool {
    match parts.split_first() {
        None => names.next().is_none(),
        // A glob that ends with `**` matches what is inside, not the directory itself.
        Some((Part::Dirs, [])) => names.next().is_some(),
        Some((Part::Dirs, rest)) => loop {
            if matches_parts(rest, names.clone()) {
                return true;
            }
            if names.next().is_none() {
                return false;
            }
        },
        Some((Part::Name(pattern), rest)) => names
            .next()
            .is_some_and(|name| fnmatch(pattern, name) && matches_parts(rest, names)),
    }
}

/// Whether `name` matches `pattern` as `fnmatch` without flags matches it: `*`, `?`, `[…]` with
/// `!` or `^` to negate, ranges and `[:class:]`, and `\` to take the next character as it is. A
/// leading dot needs no dot in the pattern, as with `find -name`.
fn fnmatch(p: &[char], name: &str) -> bool {
    let (mut pi, mut si) = (0, 0);
    let mut star = None;
    while let Some(c) = name[si..].chars().next() {
        if p.get(pi) == Some(&'*') {
            star = Some((pi, si));
            pi += 1;
        } else if pi < p.len()
            && let Some(next) = step(p, pi, c)
        {
            pi = next;
            si += c.len_utf8();
        } else if let Some((at, from)) = star {
            let skipped = name[from..].chars().next().map_or(1, char::len_utf8);
            pi = at + 1;
            si = from + skipped;
            star = Some((at, si));
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
            found |= in_class(&p[i + 2..i + 2 + len], c);
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

fn in_class(class: &[char], c: char) -> bool {
    let is = |name: &str| class.iter().copied().eq(name.chars());
    if is("alpha") {
        c.is_alphabetic()
    } else if is("digit") {
        c.is_ascii_digit()
    } else if is("alnum") {
        c.is_alphanumeric()
    } else if is("upper") {
        c.is_uppercase()
    } else if is("lower") {
        c.is_lowercase()
    } else if is("space") {
        c.is_whitespace()
    } else if is("punct") {
        c.is_ascii_punctuation()
    } else if is("xdigit") {
        c.is_ascii_hexdigit()
    } else {
        false
    }
}

/// AI-48: a pattern as the host's `rg` and `grep -E` get it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// The pattern as the model wrote it, which `rg` reads.
    pub rg: String,
    /// The same pattern for `grep -E`, where the host has no `rg`; `Err` says what in it only
    /// `rg` can run.
    pub extended: Result<String, &'static str>,
    /// Letter case is ignored: asked for, or by a `(?i)` that starts the pattern, which `grep -E`
    /// gets as `-i`.
    pub ignore_case: bool,
}

/// What only `rg` can run, for the message where the host has none.
const MULTILINE: &str = "a search across lines (multiline)";
const UNICODE_CLASS: &str = "Unicode classes such as \\p{L}";
const FLAGS: &str = "inline flags other than a (?i) that starts the pattern";
const TEXT_ANCHOR: &str = "\\A and \\z";
const HALF_BOUNDARY: &str = "\\b{start-half} and \\b{end-half}";
const CLASS_OPERATION: &str = "class operations such as [a-z&&[^aeiou]]";
const NEGATED_IN_CLASS: &str = "negated classes such as \\D or [:^alpha:] inside brackets";
const ASCII_CLASS: &str = "[:ascii:]";
const SPECIAL_RANGE: &str = "a range that starts or ends with ] - ^ or [";
const CONTROL: &str = "a carriage return or NUL";

/// AI-48: `pattern`, a regular expression as `rg` reads it (Rust's `regex` syntax), checked as
/// `rg` checks it, and translated for `grep -E` where the host has no `rg`, for the same lines:
/// `\d`, `\s` and `\w` and their negations as POSIX classes, `(?:…)` and named groups as plain
/// ones, lazy repetitions as greedy ones, which match the same lines, a leading `(?i)` as `-i`,
/// and escapes as `grep -E` reads them. An error is the message for the model.
pub fn pattern(given: &str, ignore_case: bool, multiline: bool) -> Result<Pattern, String> {
    if given.is_empty() {
        return Err("The pattern is empty, so nothing was searched.".to_owned());
    }
    if given.contains(['\n', '\r', '\0']) {
        return Err(
            "The pattern has a line break, so nothing was searched. Write a line break as \\n, \
             with multiline set to true."
                .to_owned(),
        );
    }
    let ast = ast::parse::Parser::new()
        .parse(given)
        .map_err(|e| invalid(e.kind(), e.span()))?;
    regex_syntax::hir::translate::TranslatorBuilder::new()
        .utf8(false)
        .build()
        .translate(given, &ast)
        .map_err(|e| invalid(e.kind(), e.span()))?;
    if !multiline && has_line_feed(&ast) {
        return Err(
            "The pattern matches a line break (\\n), and a search reads one line at a time unless \
             multiline is true, so nothing was searched. Set multiline to true to match across \
             lines."
                .to_owned(),
        );
    }
    let mut body = ast.clone();
    let leading_i = strip_case_flag(&mut body);
    let extended = if multiline {
        Err(MULTILINE)
    } else {
        let mut out = String::new();
        extended(&body, &mut out).map(|()| if out.is_empty() { "^".to_owned() } else { out })
    };
    Ok(Pattern {
        rg: given.to_owned(),
        extended,
        ignore_case: ignore_case || leading_i,
    })
}

fn invalid(kind: &impl Display, span: &ast::Span) -> String {
    format!(
        "The pattern is not a valid regular expression ({kind}, at character {}), so nothing was \
         searched. Escape ( ) [ ] {{ }} . * + ? | ^ $ and \\ with a backslash to match them as \
         they are.",
        span.start.column
    )
}

/// Takes out a `(?i)` that starts the pattern, also when it starts the first of its alternatives,
/// which it covers too: whether there was one. One inside a group covers only the group.
fn strip_case_flag(ast: &mut Ast) -> bool {
    match ast {
        Ast::Flags(set) if is_case_flag(&set.flags) => {
            *ast = Ast::empty(set.span);
            true
        }
        Ast::Concat(concat) => concat.asts.first_mut().is_some_and(strip_case_flag),
        Ast::Alternation(alternation) => alternation.asts.first_mut().is_some_and(strip_case_flag),
        _ => false,
    }
}

fn is_case_flag(flags: &ast::Flags) -> bool {
    matches!(
        &flags.items[..],
        [ast::FlagsItem {
            kind: ast::FlagsItemKind::Flag(ast::Flag::CaseInsensitive),
            ..
        }]
    )
}

/// Whether `ast` has a `\n` outside brackets, which `rg` refuses unless the search is multiline.
fn has_line_feed(ast: &Ast) -> bool {
    match ast {
        Ast::Literal(literal) => literal.c == '\n',
        Ast::Repetition(repetition) => has_line_feed(&repetition.ast),
        Ast::Group(group) => has_line_feed(&group.ast),
        Ast::Alternation(alternation) => alternation.asts.iter().any(has_line_feed),
        Ast::Concat(concat) => concat.asts.iter().any(has_line_feed),
        _ => false,
    }
}

/// Whether `rg -c` counts matches rather than matching lines for `args`: in a multiline search
/// whose pattern can take in a line break, or has `^`, `$`, `\A` or `\z`, for which rg reads files
/// whole, as its `non_matching_bytes` tells.
fn counts_matches(args: &GrepArgs) -> bool {
    args.multiline.unwrap_or(false)
        && regex_syntax::ParserBuilder::new()
            .multi_line(true)
            .dot_matches_new_line(true)
            .utf8(false)
            .build()
            .parse(&args.pattern)
            .is_ok_and(|hir| takes_line_break(&hir))
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

/// Appends `ast` as `grep -E` reads it.
fn extended(ast: &Ast, out: &mut String) -> Result<(), &'static str> {
    match ast {
        Ast::Empty(_) => {}
        Ast::Flags(_) => return Err(FLAGS),
        Ast::Literal(literal) => match literal.c {
            '\r' | '\0' => return Err(CONTROL),
            c @ ('.' | '[' | '\\' | '(' | ')' | '*' | '+' | '?' | '{' | '|' | '^' | '$') => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        },
        Ast::Dot(_) => out.push('.'),
        Ast::Assertion(assertion) => out.push_str(match assertion.kind {
            ast::AssertionKind::StartLine => "^",
            ast::AssertionKind::EndLine => "$",
            ast::AssertionKind::WordBoundary => "\\b",
            ast::AssertionKind::NotWordBoundary => "\\B",
            ast::AssertionKind::WordBoundaryStart | ast::AssertionKind::WordBoundaryStartAngle => {
                "\\<"
            }
            ast::AssertionKind::WordBoundaryEnd | ast::AssertionKind::WordBoundaryEndAngle => "\\>",
            ast::AssertionKind::StartText | ast::AssertionKind::EndText => {
                return Err(TEXT_ANCHOR);
            }
            ast::AssertionKind::WordBoundaryStartHalf | ast::AssertionKind::WordBoundaryEndHalf => {
                return Err(HALF_BOUNDARY);
            }
        }),
        Ast::ClassUnicode(_) => return Err(UNICODE_CLASS),
        Ast::ClassPerl(class) => out.push_str(match (&class.kind, class.negated) {
            (ast::ClassPerlKind::Digit, false) => "[0-9]",
            (ast::ClassPerlKind::Digit, true) => "[^0-9]",
            (ast::ClassPerlKind::Space, false) => "[[:space:]]",
            (ast::ClassPerlKind::Space, true) => "[^[:space:]]",
            (ast::ClassPerlKind::Word, false) => "[[:alnum:]_]",
            (ast::ClassPerlKind::Word, true) => "[^[:alnum:]_]",
        }),
        Ast::ClassBracketed(class) => {
            let mut set = Bracket {
                negated: class.negated,
                ..Bracket::default()
            };
            match &class.kind {
                ast::ClassSet::Item(item) => set.add(item)?,
                ast::ClassSet::BinaryOp(_) => return Err(CLASS_OPERATION),
            }
            set.write(out)?;
        }
        Ast::Repetition(repetition) => {
            // A lazy repetition matches the same lines as a greedy one.
            extended(&repetition.ast, out)?;
            match &repetition.op.kind {
                ast::RepetitionKind::ZeroOrOne => out.push('?'),
                ast::RepetitionKind::ZeroOrMore => out.push('*'),
                ast::RepetitionKind::OneOrMore => out.push('+'),
                ast::RepetitionKind::Range(ast::RepetitionRange::Exactly(n)) => {
                    out.push_str(&format!("{{{n}}}"));
                }
                ast::RepetitionKind::Range(ast::RepetitionRange::AtLeast(n)) => {
                    out.push_str(&format!("{{{n},}}"));
                }
                ast::RepetitionKind::Range(ast::RepetitionRange::Bounded(m, n)) => {
                    out.push_str(&format!("{{{m},{n}}}"));
                }
            }
        }
        Ast::Group(group) => {
            if group.flags().is_some_and(|flags| !flags.items.is_empty()) {
                return Err(FLAGS);
            }
            out.push('(');
            extended(&group.ast, out)?;
            out.push(')');
        }
        Ast::Alternation(alternation) => {
            for (i, ast) in alternation.asts.iter().enumerate() {
                if i > 0 {
                    out.push('|');
                }
                extended(ast, out)?;
            }
        }
        Ast::Concat(concat) => {
            for ast in &concat.asts {
                extended(ast, out)?;
            }
        }
    }
    Ok(())
}

/// The members of a bracket expression, gathered to be written as POSIX reads them.
#[derive(Default)]
struct Bracket {
    negated: bool,
    chars: BTreeSet<char>,
    ranges: Vec<(char, char)>,
    classes: Vec<&'static str>,
}

impl Bracket {
    fn add(&mut self, item: &ast::ClassSetItem) -> Result<(), &'static str> {
        match item {
            ast::ClassSetItem::Empty(_) => {}
            // A line break is never part of a line, as `rg` takes it out of a class.
            ast::ClassSetItem::Literal(literal) if literal.c == '\n' => {}
            ast::ClassSetItem::Literal(literal) => {
                if matches!(literal.c, '\r' | '\0') {
                    return Err(CONTROL);
                }
                self.chars.insert(literal.c);
            }
            ast::ClassSetItem::Range(range) => {
                let ends = [range.start.c, range.end.c];
                if ends.iter().any(|c| matches!(c, ']' | '-' | '^' | '[')) {
                    return Err(SPECIAL_RANGE);
                }
                if ends.iter().any(|c| matches!(c, '\n' | '\r' | '\0')) {
                    return Err(CONTROL);
                }
                self.ranges.push((range.start.c, range.end.c));
            }
            ast::ClassSetItem::Ascii(class) => {
                if class.negated {
                    return Err(NEGATED_IN_CLASS);
                }
                self.classes.push(match class.kind {
                    ast::ClassAsciiKind::Alnum => "alnum",
                    ast::ClassAsciiKind::Alpha => "alpha",
                    ast::ClassAsciiKind::Blank => "blank",
                    ast::ClassAsciiKind::Cntrl => "cntrl",
                    ast::ClassAsciiKind::Digit => "digit",
                    ast::ClassAsciiKind::Graph => "graph",
                    ast::ClassAsciiKind::Lower => "lower",
                    ast::ClassAsciiKind::Print => "print",
                    ast::ClassAsciiKind::Punct => "punct",
                    ast::ClassAsciiKind::Space => "space",
                    ast::ClassAsciiKind::Upper => "upper",
                    ast::ClassAsciiKind::Xdigit => "xdigit",
                    ast::ClassAsciiKind::Word => {
                        self.chars.insert('_');
                        "alnum"
                    }
                    ast::ClassAsciiKind::Ascii => return Err(ASCII_CLASS),
                });
            }
            ast::ClassSetItem::Unicode(_) => return Err(UNICODE_CLASS),
            ast::ClassSetItem::Perl(class) => {
                if class.negated {
                    return Err(NEGATED_IN_CLASS);
                }
                match class.kind {
                    ast::ClassPerlKind::Digit => self.ranges.push(('0', '9')),
                    ast::ClassPerlKind::Space => self.classes.push("space"),
                    ast::ClassPerlKind::Word => {
                        self.classes.push("alnum");
                        self.chars.insert('_');
                    }
                }
            }
            ast::ClassSetItem::Bracketed(inner) => match &inner.kind {
                _ if inner.negated => return Err(NEGATED_IN_CLASS),
                ast::ClassSet::Item(item) => self.add(item)?,
                ast::ClassSet::BinaryOp(_) => return Err(CLASS_OPERATION),
            },
            ast::ClassSetItem::Union(union) => {
                for item in &union.items {
                    self.add(item)?;
                }
            }
        }
        Ok(())
    }

    /// POSIX gives `]` a place first, `-` last and `^` anywhere but first; `\` stands for itself,
    /// and `[` for itself unless `:`, `.` or `=` follows.
    fn write(&self, out: &mut String) -> Result<(), &'static str> {
        let has = |c| self.chars.contains(&c);
        let mut body = String::new();
        if has(']') {
            body.push(']');
        }
        for &c in &self.chars {
            if !matches!(c, ']' | '-' | '^' | '[') {
                body.push(c);
            }
        }
        for &(low, high) in &self.ranges {
            body.extend([low, '-', high]);
        }
        for class in &self.classes {
            body.push_str(&format!("[:{class}:]"));
        }
        if has('[') {
            body.push('[');
        }
        let mut dash = has('-');
        if has('^') {
            if !body.is_empty() || self.negated {
                body.push('^');
            } else if dash {
                body.push_str("-^");
                dash = false;
            } else {
                // `[^]` would be a negation.
                out.push_str("\\^");
                return Ok(());
            }
        }
        if dash {
            body.push('-');
        }
        if body.is_empty() {
            // Only line breaks, which no line has: negated, any character of the line.
            if self.negated {
                out.push('.');
                return Ok(());
            }
            return Err(CONTROL);
        }
        out.push('[');
        if self.negated {
            out.push('^');
        }
        out.push_str(&body);
        out.push(']');
        Ok(())
    }
}

/// The lines of context before and after each matching line that `args` ask for: `-B` and `-A`,
/// or `context` for either, at most [`MAX_CONTEXT`] each.
#[must_use]
pub fn context(args: &GrepArgs) -> (u32, u32) {
    let lines = |side: Option<u64>| {
        u32::try_from(side.or(args.context).unwrap_or(0).min(MAX_CONTEXT)).unwrap_or_default()
    };
    (lines(args.before_context), lines(args.after_context))
}

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
struct Problems {
    denied: usize,
    other: Vec<String>,
}

fn problems(stderr: &[u8]) -> Problems {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn search(path: Option<&str>, glob: &str) -> Search {
        Search::new(path, Some(glob)).unwrap()
    }

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn fnm(pattern: &str, name: &str) -> bool {
        fnmatch(&chars(pattern), name)
    }

    #[test]
    fn globs_match_names_at_any_depth_and_paths_from_the_start() {
        // No `/`: a name at any depth, as with rg's --glob and find -name.
        let s = search(Some("/etc/nginx"), "*.conf");
        assert_eq!(
            (s.path.as_str(), s.below.as_str(), s.max_depth, &s.names[..]),
            ("/etc/nginx", "", None, &["*.conf".to_owned()][..])
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
        assert_eq!(
            (s.path.as_str(), s.below.as_str(), s.max_depth),
            ("/srv", "app", None)
        );
        for yes in [".env", "x.env", "config/prod/x.env"] {
            assert!(s.matches(yes), "{yes}");
        }
        let s = search(Some("/srv"), "logs/**");
        assert!(s.matches("a") && s.matches("a/b") && s.names.is_empty());
    }

    #[test]
    fn globs_start_where_they_name_directories() {
        // The directories a glob starts with are searched below the path, and the result names
        // the directory the model gave.
        let s = search(Some("/etc/"), "nginx/sites-enabled/*");
        assert_eq!(
            (s.path.as_str(), s.below.as_str(), s.max_depth),
            ("/etc/", "nginx/sites-enabled", Some(1))
        );
        assert_eq!(s.shown(), "/etc/nginx/sites-enabled");
        assert!(s.matches("api") && !s.matches("a/b"));
        assert_eq!(s.base("/etc/nginx/sites-enabled"), "/etc");
        // An absolute glob searches from /, whatever the path.
        let s = search(Some("/home/ops"), "/etc/nginx/**/*.conf");
        assert_eq!(
            (s.path.as_str(), s.below.as_str(), s.max_depth),
            ("/", "etc/nginx", None)
        );
        assert!(s.matches("nginx.conf") && s.matches("conf.d/a.conf"));
        assert_eq!(
            (s.shown(), s.base("/etc/nginx")),
            ("/etc/nginx".into(), "/".into())
        );
        let s = search(None, "/*.conf");
        assert_eq!((s.path.as_str(), s.max_depth), ("/", Some(1)));
        assert!(s.matches("x.conf") && !s.matches("etc/x.conf"));
        // `./` and a trailing `/` change nothing; a directory that is a pattern stays in it.
        assert_eq!(search(None, "./a/b/").shown(), "~/a");
        assert_eq!(search(Some("/"), "{etc,opt}/*.conf").shown(), "/");
        // A file given as the path is searched itself, and a path the base cannot be found in is
        // shown as resolved.
        assert_eq!(search(None, "a/b").base("/srv/notes.txt"), "/srv/notes.txt");
    }

    #[test]
    fn a_glob_from_a_drive_searches_from_its_root() {
        // Windows paths (§7.5): whatever the path, as with a glob that starts with /.
        let s = search(Some("C:/Users/kc/missing.txt"), "c:/Projects/**/*.json");
        assert_eq!(
            (s.path.as_str(), s.below.as_str(), s.max_depth),
            ("C:/", "Projects", None)
        );
        assert_eq!(
            (s.shown(), s.base("C:/Projects")),
            ("C:/Projects".into(), "C:/".into())
        );
        let s = search(None, "D:/*.log");
        assert_eq!((s.path.as_str(), s.below.as_str()), ("D:/", ""));
        assert!(s.matches("a.log") && !s.matches("logs/a.log"));
        assert_eq!(Root::of("D:/.").place(&s), "under D:/");
        assert_eq!(
            Root::of("D:/.").kept("D:/./a.log", &s).as_deref(),
            Some("D:/a.log")
        );
        let s = search(None, "C:/Projects/{a,b}/*.json");
        assert_eq!((s.path.as_str(), s.below.as_str()), ("C:/", ""));
        assert!(s.matches("Projects/b/x.json") && !s.matches("Projects/c/x.json"));
        // Without the root's slash, `C:` names a directory.
        assert_eq!(search(None, "C:*.log").path, "~");
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

        assert!(fnm("[a-c]?.log", "b1.log") && !fnm("[a-c]?.log", "d1.log"));
        assert!(fnm("[!.]*", "x") && !fnm("[^.]*", ".x"));
        assert!(fnm("[[:digit:]]*", "9x") && fnm("[]x]", "]"));
        assert!(fnm(r"\*.txt", "*.txt") && !fnm(r"\*.txt", "a.txt"));
        assert!(fnm("[", "[") && fnm("*", ".bashrc") && fnm("a*b*c", "aXbYbc"));
        assert!(!fnm("a*b", "acb/") && fnm("日*", "日本") && fnm("*本", "日本"));
        assert!(fnm("?本", "日本") && !fnm("[[:alpha:]]x", "1x"));

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
    fn names_mean_the_same_to_find_and_rg() {
        let host = |name: &str| host_name(&chars(name));
        // find reads [! everywhere; [^ only some find do.
        assert_eq!(host("[^.]*"), "[!.]*");
        assert_eq!(host(r"\[^x]"), r"\[^x]");
        // What rg reads as its own: braces, a leading ! or #, spaces at the end; ** it refuses.
        assert_eq!(host("{x}.conf"), r"\{x\}.conf");
        assert_eq!(host("!x"), r"\!x");
        assert_eq!(host("#x#"), r"\#x#");
        assert_eq!(host("a b  "), r"a b\ \ ");
        assert_eq!(host("a**b"), "a*b");
        assert_eq!(host(r"a\**"), r"a\**");
        // A class rg does not know widens to any character; a lone [ or \ is escaped.
        assert_eq!(host("[[:digit:]]*.log"), "?*.log");
        assert_eq!(host("x[ab"), r"x\[ab");
        assert_eq!(host(r"end\"), r"end\\");
        assert_eq!(host("[]x]"), "[]x]");
        let s = search(None, "[[:digit:]]*.{log,txt}");
        assert_eq!(s.names, ["?*.log", "?*.txt"]);
        assert!(s.matches("9.log") && !s.matches("x.log"));
    }

    fn ere(p: &str) -> String {
        pattern(p, false, false).unwrap().extended.unwrap()
    }

    #[test]
    fn patterns_read_alike_on_rg_and_grep() {
        assert_eq!(ere(r"listen\s+\d+;"), "listen[[:space:]]+[0-9]+;");
        assert_eq!(
            ere(r"\w+\W\S\D"),
            "[[:alnum:]_]+[^[:alnum:]_][^[:space:]][^0-9]"
        );
        assert_eq!(ere(r"[\d.]+:[\w-]"), "[.0-9]+:[_[:alnum:]-]");
        assert_eq!(ere(r"a\tb\x41\u{e9}"), "a\tbAé");
        // Escapes as grep -E reads them; ], } and other punctuation need none.
        assert_eq!(
            ere(r"\.\(\)\[\]\{\}\|\^\$\*\+\?\\"),
            r"\.\(\)\[]\{}\|\^\$\*\+\?\\"
        );
        assert_eq!(ere(r"a\/b\-c\#d\:e\ f\<"), r"a/b-c#d:e f\<");
        assert_eq!(ere(r"\bx\B\b{start}y\b{end}"), r"\bx\B\<y\>");
        // Groups, lazy repetitions, counted repetitions and a leading (?i).
        assert_eq!(
            ere(r"(?:a|b)(?P<n>c)+?x{2}y{1,}z{0,3}?"),
            "(a|b)(c)+x{2}y{1,}z{0,3}"
        );
        // A leading (?i) covers every alternative, as rg reads it; one later covers only the rest.
        let p = pattern("(?i)error|warn", false, false).unwrap();
        assert_eq!(
            (p.extended, p.ignore_case),
            (Ok("error|warn".to_owned()), true)
        );
        assert!(
            regex_syntax::parse("(?i)a|b")
                .unwrap()
                .to_string()
                .contains('B')
        );
        for later in ["a|(?i)b", "((?i)a)b"] {
            assert_eq!(pattern(later, false, false).unwrap().extended, Err(FLAGS));
        }
        let p = pattern("(?i)error", false, false).unwrap();
        assert_eq!(
            (p.extended.unwrap(), p.ignore_case, p.rg.as_str()),
            ("error".to_owned(), true, "(?i)error")
        );
        // Brackets as POSIX reads them: ] first, - last, ^ not first, [ and \ as they are.
        assert_eq!(ere(r"[\]\-a\^]"), "[]a^-]");
        assert_eq!(ere(r"[\^\-]"), "[-^]");
        assert_eq!(ere(r"[\^]"), r"\^");
        assert_eq!(ere(r"[^\^]"), "[^^]");
        assert_eq!(ere(r"[\[:]"), "[:[]");
        assert_eq!(ere(r"[\\a]"), r"[\a]");
        assert_eq!(ere("[[:alpha:][:word:]x]"), "[_x[:alpha:][:alnum:]]");
        assert_eq!(ere(r"[a-z[0-9]]"), "[a-z0-9]");
        assert_eq!(ere("日本"), "日本");
        // What only rg can run.
        for (p, why) in [
            (r"\p{L}+", UNICODE_CLASS),
            (r"a(?s:.)b", FLAGS),
            (r"(?-i)x", FLAGS),
            (r"\Ax", TEXT_ANCHOR),
            (r"[a-z&&[^aeiou]]", CLASS_OPERATION),
            (r"[\D]", NEGATED_IN_CLASS),
            (r"[[:ascii:]]", ASCII_CLASS),
            (r"\r", CONTROL),
        ] {
            assert_eq!(pattern(p, false, false).unwrap().extended, Err(why), "{p}");
        }
        assert_eq!(pattern("x", false, true).unwrap().extended, Err(MULTILINE));
    }

    #[test]
    fn patterns_are_checked_as_rg_checks_them() {
        for (p, message) in [
            ("(", "unclosed group"),
            (r"\1", "backreferences are not supported"),
            ("(?=x)", "look-around"),
            (r"\p{Nope}", "Unicode property not found"),
            ("a{", "repetition"),
        ] {
            let error = pattern(p, false, false).unwrap_err();
            assert!(error.contains(message), "{p}: {error}");
            assert!(error.starts_with("The pattern is not a valid regular expression"));
        }
        assert!(pattern("", false, false).unwrap_err().contains("empty"));
        assert!(
            pattern("a\nb", false, true)
                .unwrap_err()
                .contains("line break")
        );
        // \n matches across lines only with multiline, as with rg.
        assert!(
            pattern(r"a\nb", false, false)
                .unwrap_err()
                .contains("multiline")
        );
        let p = pattern(r"struct \{[\s\S]*?field", false, true).unwrap();
        assert_eq!(p.rg, r"struct \{[\s\S]*?field");
        // A class with \n leaves it out, as rg does: a negated one of \n alone is any character.
        assert_eq!(ere(r"[\na]"), "[a]");
        assert_eq!(ere(r"a[^\n]*b"), "a.*b");
        assert_eq!(
            pattern(r"[\n]", false, false).unwrap().extended,
            Err(CONTROL)
        );
    }

    fn found<'a>(start: &'a str, stdout: &'a str, stderr: &'a str) -> Found<'a> {
        Found {
            start,
            stdout: stdout.as_bytes(),
            nul: false,
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
    fn denials_are_counted_once_whoever_reports_them() {
        let stderr = "find: ‘/x/./locked’: Permission denied\n\n\
                      rg: /x/./locked: Permission denied (os error 13)\n\
                      rg: /x/./locked: Permission denied (os error 13)\n\
                      grep: /x/./key: Permission denied\n\
                      rg: /x/./gone.conf: No such file or directory (os error 2)\n\
                      rg: /x/./loop: File system loop found (os error 40)\n";
        let problems = problems(stderr.as_bytes());
        assert_eq!(problems.denied, 2);
        assert_eq!(
            problems.other,
            ["rg: /x/./loop: File system loop found (os error 40)"]
        );
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
        // head_limit, Claude Code's name for limit, with 0 for no limit.
        let many: String = (0..300).map(|i| format!("/x/./f{i:03}\n")).collect();
        let all = grep_result(
            &grep(r#"{"pattern":"x","head_limit":0}"#),
            &search(Some("/x"), ""),
            &found("/x/.", &many, ""),
        );
        assert!(all.starts_with("300 files with a matching line under /x, sorted by path:\n"));
        let some = grep_result(
            &grep(r#"{"pattern":"x","head_limit":2}"#),
            &search(Some("/x"), ""),
            &found("/x/.", &many, ""),
        );
        assert!(some.contains("; files 1–2:\n"), "{some}");
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
        let expected = "4 matching lines in 2 files under /etc/nginx, sorted by path:\n\
                        /etc/nginx/a-2:3:x\n/etc/nginx/a-2:4:8080\n--\n/etc/nginx/sites/b-1-server \
                        {\n/etc/nginx/sites/b:2:  listen 8080;\n/etc/nginx/sites/b-3-}\n--\n\
                        /etc/nginx/sites/b-9-x\n/etc/nginx/sites/b:10:  listen 8080;\n";
        assert_eq!(out, expected);
        // The same from rg or grep --null, and with Claude Code's -C, -B and -A.
        let nul = stdout
            .replace("/b-", "/b\0")
            .replace("/b:", "/b\0")
            .replace("a-2:", "a-2\0");
        let mut with_nul = found("/etc/nginx/.", &nul, "");
        with_nul.nul = true;
        for args in [
            r#"{"pattern":"8080","output_mode":"content","-C":1}"#,
            r#"{"pattern":"8080","output_mode":"content","-B":1,"-A":1}"#,
            r#"{"pattern":"8080","output_mode":"content","-B":1}"#,
        ] {
            assert_eq!(grep_result(&grep(args), &s, &with_nul), expected, "{args}");
        }
        // Without line numbers, as Claude Code's -n false asks.
        let out = grep_result(
            &grep(r#"{"pattern":"8080","output_mode":"content","-n":false,"context":1}"#),
            &s,
            &with_nul,
        );
        assert!(
            out.contains("\n/etc/nginx/a-2:x\n/etc/nginx/a-2:8080\n--\n"),
            "{out}"
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
    fn names_with_line_numbers_in_them_are_read_whole_with_nul() {
        // `b:2:c` is a file, and `a-2` follows `a` where two runs of grep meet.
        let stdout = "/s/./b:2:c\x001:x\n/s/./a\x001:x\n/s/./a\x002-y\n/s/./a-2\x001:x\n\
                      /s/./bin: binary file matches (found \"\\0\" byte around offset 3)\n--\n";
        let mut out = found("/s/.", stdout, "");
        out.nul = true;
        let text = grep_result(
            &grep(r#"{"pattern":"x","output_mode":"content","-A":1}"#),
            &search(Some("/s"), ""),
            &out,
        );
        assert_eq!(
            text,
            "3 matching lines in 3 files under /s, sorted by path:\n/s/a:1:x\n/s/a-2-y\n--\n\
             /s/a-2:1:x\n--\n/s/b:2:c:1:x\n"
        );
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
        // A multiline match is its lines, each a matching one.
        let mut lines = found("/s/a", "/s/a\x001:x\n/s/a\x002:y\n", "");
        lines.nul = true;
        let out = grep_result(
            &grep(r#"{"pattern":"x\\ny","output_mode":"content","multiline":true}"#),
            &Search::new(Some("/s/a"), None).unwrap(),
            &lines,
        );
        assert_eq!(
            out,
            "2 matching lines in 1 file in /s/a, sorted by path:\n/s/a:1:x\n/s/a:2:y\n"
        );
    }

    #[test]
    fn a_multiline_count_names_what_rg_counts() {
        let s = Search::new(Some("/s/a"), None).unwrap();
        let counted = found("/s/a", "/s/a:3\n", "");
        // `rg -c -U` counts matches where the pattern can span lines, and lines otherwise.
        for (json, summary) in [
            (r#"{"pattern":"x\\ny","multiline":true}"#, "3 matches"),
            (r#"{"pattern":"a.b","multiline":true}"#, "3 matches"),
            (r#"{"pattern":"^x","multiline":true}"#, "3 matches"),
            (r#"{"pattern":"ab","multiline":true}"#, "3 matching lines"),
            (
                r#"{"pattern":"a(?-s:.)","multiline":true}"#,
                "3 matching lines",
            ),
            (r#"{"pattern":"^x"}"#, "3 matching lines"),
        ] {
            let mut args = grep(json);
            args.output_mode = Some(GrepOutput::Count);
            assert_eq!(
                grep_result(&args, &s, &counted),
                format!("{summary} in 1 file in /s/a, sorted by path:\n/s/a:3\n"),
                "{json}"
            );
        }
    }
}

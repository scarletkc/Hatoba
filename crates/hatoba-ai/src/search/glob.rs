/// Alternatives the braces of a glob may expand to.
const MAX_ALTERNATIVES: usize = 64;

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
    pub(super) fn base(&self, root: &str) -> String {
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
pub(super) fn is_drive(text: &str) -> bool {
    matches!(text.as_bytes(), [letter, b':'] if letter.is_ascii_alphabetic())
}

/// The alternatives of a glob's braces, `{a,b}`, nested or not; braces without a comma, or
/// without a closing brace, stand for themselves.
pub(super) fn expand_braces(glob: &str) -> Result<Vec<String>, String> {
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
pub(super) fn host_name(name: &[char]) -> String {
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
pub(super) fn fnmatch(p: &[char], name: &str) -> bool {
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

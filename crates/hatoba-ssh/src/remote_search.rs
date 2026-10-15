//! Searching the host's files over an exec channel (spec §13.4, "Search tools"): the `find`, and
//! the `rg` or `grep`, that the AI assistant's `glob` and `grep` run (AI-48, AI-49).
//!
//! Like the file tools' scripts ([`crate::remote_file`]), each is a fixed `sh -c` script whose
//! arguments travel on stdin, one per line, so none goes through the quoting of the user's login
//! shell. So does the helper that `find -exec` runs on symbolic links, which may hold quotes. They
//! use POSIX `find` with `-maxdepth`, and `head -c`, which GNU, BSD and BusyBox share. A directory
//! is searched from `dir/.`, so a symbolic link to one is searched too, and every path printed
//! starts with that. Below it, only files and symbolic links to files are listed and searched,
//! checked with GNU `find -xtype f`, or by the helper, one `sh` for many links, where `find` lacks
//! it: links to directories are not followed, and a FIFO, which grep would wait on, is left out.
//! `.git` is skipped, and so are `/proc`, `/sys`, `/dev` and `/run` when the search starts at `/`.
//! No `-exec … +` is inside parentheses, where BusyBox `find` never runs it: the name tests are
//! repeated instead.
//!
//! `grep` runs `rg` where the host has one that knows the options it needs, and `grep -E`
//! otherwise, on the pattern as Hatoba translated it for `grep -E`. `find` lists the directories
//! and `rg` searches the files in each (`--max-depth 1`, following links), so it searches the files
//! `grep -E` would and skips binary files as it skips them in a directory. `grep` gets `-I` and
//! `--null` where it has them, and checks the pattern on an empty line, since BusyBox `grep`
//! compiles it only once it reads one.
//!
//! Messages are in English (`LC_MESSAGES=C`, with an `LC_ALL` moved to the character and collation
//! categories, where it would override that), so the caller can count the files that could not be
//! read. sshd does not signal a command without a PTY when its channel closes, so the search runs
//! under [`RUNNER`], whose watchdog ends it once the channel is closed or the time limit passes,
//! should the connection die without the channel closing. It signals only the search: never the
//! script's own process group, which Dropbear, unlike OpenSSH, shares with the connection.

use std::time::Duration;

use crate::error::SshError;
use crate::exec::Exec;
use crate::remote_file::{after_marker, command, last_line};
use crate::session::ExecOutput;

/// Reads the path, and the directories below it to search instead, from the variables `f` and `s`,
/// and resolves them as the file tools resolve a path, `s` only when `f` is a directory. Exit
/// statuses: 2 missing, 8 a symbolic link to nothing.
macro_rules! resolve {
    () => {
        "case $f in \"~\") f=$HOME;; \"~/\"*) f=$HOME/${f#\"~/\"};; /*) ;; *) f=$HOME/$f;; esac; \
         if [ -n \"$s\" ] && [ -d \"$f\" ]; then f=${f%/}/$s; fi; \
         [ -L \"$f\" ] && { [ -e \"$f\" ] || exit 8; }; [ -e \"$f\" ] || exit 2; "
    };
}

/// English messages, whatever `LC_ALL` says.
macro_rules! english {
    () => {
        "[ -z \"$LC_ALL\" ] || { LC_CTYPE=$LC_ALL; LC_COLLATE=$LC_ALL; \
         export LC_CTYPE LC_COLLATE; unset LC_ALL; }; LC_MESSAGES=C; export LC_MESSAGES; "
    };
}

/// The `find -name` tests for the names on the rest of stdin, any of which a file may match.
macro_rules! find_names {
    () => {
        "while IFS= read -r n; do \
         if [ $# -eq 0 ]; then set -- -name \"$n\"; else set -- \"$@\" -o -name \"$n\"; fi; done; \
         [ $# -eq 0 ] || set -- \"(\" \"$@\" \")\"; "
    };
}

/// Puts before the `find` expression the directories it skips and the depth `$d` (`-` for any).
macro_rules! prune {
    () => {
        "[ \"$r\" = /. ] && set -- \"(\" -path /./proc -o -path /./sys -o -path /./dev \
         -o -path /./run \")\" -prune -o \"$@\"; set -- -name .git -prune -o \"$@\"; \
         [ \"$d\" = - ] || set -- -maxdepth \"$d\" \"$@\"; "
    };
}

/// Runs the search, its arguments, as the scripts end with it, `exec`ed in their place: its output
/// goes through `$y` (a filter, or `cat`) and `head -c $c`, and a watchdog ends it once stderr is
/// closed or `$t` seconds have passed. It travels on stdin, so it may use `$!`, which the scripts
/// may not: the search writes to a FIFO, so `$!` is its process. That is in a process group of its
/// own where `setsid` (util-linux and BusyBox) or `perl`'s `setpgrp` (macOS and the BSDs) can make
/// one, and the watchdog sends the group TERM. Where neither can, it sends TERM to the search's
/// process and those it started, such as the `rg` of a `find -exec`, found by their parents in
/// `/proc`, or with `ps` where there is none. The script's own process group is never signalled.
/// When the output is done, the watchdog is stopped and the search ended. The watchdog's `sleep`
/// holds none of the channel's pipes, which would keep the channel open. Without `mktemp -d` or
/// `mkfifo`, the search runs without a watchdog.
const RUNNER: &str = "if setsid true 2>/dev/null; then g=setsid; \
     elif perl -e 1 2>/dev/null; then g=perl; else g=none; fi; \
     d=$(mktemp -d 2>/dev/null) && mkfifo \"$d/o\" 2>/dev/null || \
     { [ -z \"$d\" ] || rm -rf \"$d\"; \"$@\" | $y | head -c \"$c\"; exit 0; }; \
     case $g in setsid) setsid \"$@\" > \"$d/o\" & ;; \
     perl) perl -e 'setpgrp(0,0); exec @ARGV or exit 127' \"$@\" > \"$d/o\" & ;; \
     *) \"$@\" > \"$d/o\" & ;; esac; s=$!; \
     l() { if [ -r /proc/self/stat ]; then for q in /proc/[0-9]*/stat; do \
     read -r p < \"$q\" 2>/dev/null || continue; set -- ${p##*\") \"}; echo \"${p%% *} $2\"; done; \
     else ps -A -o pid= -o ppid= 2>/dev/null; fi; }; \
     e() { if [ $g = none ]; then x=\" $s \"; m=1; l > \"$d/p\"; while [ $m = 1 ]; do m=0; \
     while read -r a b; do case $x in *\" $b \"*) case $x in *\" $a \"*) ;; \
     *) x=\"$x$a \"; m=1;; esac;; esac; done < \"$d/p\"; done; kill -TERM $x 2>/dev/null; \
     else kill -TERM -$s $s 2>/dev/null; fi; }; \
     $y < \"$d/o\" | head -c \"$c\" & h=$!; \
     ( trap 'e; exit 0' PIPE; n=0; \
     while sleep 1 < /dev/null > /dev/null 2>&1 & wait $!; do n=$((n + 1)); \
     [ $n -lt \"$t\" ] || break; echo >&2 || break; done; e ) > /dev/null & w=$!; \
     wait $h; kill $w 2>/dev/null; e; wait; rm -rf \"$d\"; exit 0";

/// Lists the files, and the symbolic links to files, below the directory named on stdin's first
/// line (with stdin's second line, when not empty, appended), at most as many levels down as its
/// third line says (`-` for any), whose names match one of the `find -name` patterns on the lines
/// after the fifth, [`GLOB_LINKS`] (any name without one). Prints the marker, the directory with
/// `/.` after it, and at most `$1` bytes of `find`'s output, run by [`RUNNER`], stdin's fifth line,
/// which ends it after `$2` seconds. Exit statuses as for [`resolve!`], and 4 not readable, 6 not a
/// directory (the path as resolved on stderr), 9 no path.
const GLOB_SCRIPT: &str = concat!(
    "c=$1; t=$2; IFS= read -r f || exit 9; IFS= read -r s || exit 9; IFS= read -r d || exit 9; \
     IFS= read -r h || exit 9; IFS= read -r z || exit 9; ",
    resolve!(),
    english!(),
    "[ -d \"$f\" ] || { printf %s \"$f\" >&2; exit 6; }; \
     [ -r \"$f\" ] && [ -x \"$f\" ] || exit 4; case $f in */) r=$f.;; *) r=$f/.;; esac; set --; ",
    find_names!(),
    "if find \"$r\" -maxdepth 0 -xtype f > /dev/null 2>&1; then set -- \"$@\" -xtype f -print; \
     else set -- \"$@\" -type f -print -o \"$@\" -type l -exec sh -c \"$h\" sh {} +; fi; ",
    prune!(),
    "y=cat; export c t y; echo hatoba-file; printf %s \"$r\"; echo; set -- find \"$r\" \"$@\"; \
     exec sh -c \"$z\" sh \"$@\""
);

/// What `find -exec` runs on symbolic links for the glob script where it lacks `-xtype`: prints
/// those that are files.
const GLOB_LINKS: &str = "for f do if [ -f \"$f\" ]; then printf '%s\\n' \"$f\"; fi; done";

/// Searches the file, or the files below the directory, named on stdin's first lines as the glob
/// script reads them, the third the depth. Stdin's fourth line is the output (`l` the files, `c`
/// the counts of those with a match, `n` the lines), its fifth `i` to ignore case, its sixth and
/// seventh the lines of context before and after, its eighth `u` for a multiline search, its ninth
/// [`GREP_LINKS`], its tenth [`RUNNER`], its eleventh the pattern for `rg` and its twelfth the
/// pattern for `grep -E` (empty when only `rg` can run it). The `find -name` patterns, which `rg`
/// reads as globs, follow, and a file is searched whatever its name. Prints the marker, where
/// `find` started (the directory with `/.` after it, or the file), the matcher (`rg` or `grep`,
/// with ` --null` when each line's file is followed by a NUL) and at most `$1` bytes of its
/// output, run as the glob script runs `find`. Exit statuses as for the glob script, without 6,
/// and 10 when `grep -E` refuses the pattern, 11 when only `rg` could run the search and the host
/// has none, 12 when `rg` refuses the pattern, with their message on stderr.
const GREP_SCRIPT: &str = concat!(
    "c=$1; t=$2; IFS= read -r f || exit 9; IFS= read -r s || exit 9; IFS= read -r d || exit 9; \
     IFS= read -r m || exit 9; IFS= read -r i || exit 9; IFS= read -r b || exit 9; \
     IFS= read -r a || exit 9; IFS= read -r u || exit 9; IFS= read -r h || exit 9; \
     IFS= read -r z || exit 9; IFS= read -r q || exit 9; IFS= read -r p || exit 9; ",
    resolve!(),
    english!(),
    "[ -r \"$f\" ] || exit 4; o=; [ \"$i\" = i ] && o=-i; \
     k=\"--no-config -L --hidden --no-ignore --max-depth 1 --type-add gitlink:.git -T gitlink \
     --color never -H --no-heading\"; \
     if rg $k -U --multiline-dotall --null -n -e x /dev/null > /dev/null 2>&1; [ $? -eq 1 ]; \
     then [ \"$u\" = u ] && o=\"$o -U --multiline-dotall\"; \
     rg $k $o -e \"$q\" /dev/null > /dev/null; [ $? -lt 2 ] || exit 12; o=\"$k $o\"; p=$q; x=rg; \
     else [ \"$u\" = u ] && exit 11; [ -n \"$p\" ] || exit 11; \
     echo | grep -E -e \"$p\" > /dev/null; [ $? -lt 2 ] || exit 10; o=\"-EH $o\"; x=grep; \
     grep -I -e x /dev/null 2> /dev/null; [ $? -eq 1 ] && o=\"$o -I\"; fi; g=$x; \
     case $m in l) o=\"$o -l\";; c) o=\"$o -c\";; *) o=\"$o -n\"; \
     [ \"$b$a\" = 00 ] || o=\"$o -B $b -A $a\"; \
     if [ $x = rg ]; then o=\"$o --null\"; g=\"rg --null\"; \
     else grep --null -e x /dev/null 2> /dev/null; \
     [ $? -eq 1 ] && o=\"$o --null\" && g=\"grep --null\"; fi;; esac; \
     export o p; r=$f; set --; \
     if [ -d \"$f\" ]; then [ -x \"$f\" ] || exit 4; case $f in */) r=$f.;; *) r=$f/.;; esac; \
     if [ $x = rg ]; then while IFS= read -r n; do set -- \"$@\" -g \"$n\"; done; \
     set -- -type d -exec rg $o \"$@\" -e \"$p\" {} +; [ \"$d\" = - ] || d=$((d - 1)); else ",
    find_names!(),
    "if find \"$r\" -maxdepth 0 -xtype f > /dev/null 2>&1; then \
     set -- \"$@\" -xtype f -exec grep $o -e \"$p\" {} +; \
     else set -- \"$@\" -type f -exec grep $o -e \"$p\" {} + -o \"$@\" -type l \
     -exec sh -c \"$h\" sh {} +; fi; fi; ",
    prune!(),
    "fi; y=cat; [ $m = c ] && y=\"grep -v -e :0$\"; export c t y; \
     echo hatoba-file; printf %s \"$r\"; echo; echo \"$g\"; \
     if [ -d \"$f\" ]; then set -- find \"$r\" \"$@\"; elif [ -f \"$f\" ]; then \
     set -- $x $o -e \"$p\" \"$f\"; else set -- true; fi; exec sh -c \"$z\" sh \"$@\""
);

/// What `find -exec` runs on symbolic links for the grep script where it lacks `-xtype`: `grep`,
/// with the options and pattern the script exported, on those that are files.
const GREP_LINKS: &str = "for f do shift; if [ -f \"$f\" ]; then set -- \"$@\" \"$f\"; fi; done; \
                          [ $# -eq 0 ] || exec grep $o -e \"$p\" \"$@\"";

/// Where a search looks.
#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    /// The file or directory, as the model gave it.
    pub path: &'a str,
    /// The directories below `path` to search instead, as `a/b`, when `path` is a directory; empty
    /// for none. A file given as `path` is searched itself.
    pub below: &'a str,
    /// How many levels below the directory to search; any without.
    pub max_depth: Option<u32>,
    /// Patterns, one of which each file's name matches; any name without one. Each must mean the
    /// same to `find -name` and to `rg --glob`.
    pub names: &'a [String],
}

/// What `grep` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrepMode {
    /// The files with a matching line (`-l`).
    Files,
    /// The number of matching lines of each file with one (`-c`).
    Count,
    /// The matching lines, numbered, with this many lines of context before and after each.
    Lines { before: u32, after: u32 },
}

/// A search with `rg` or `grep`.
#[derive(Clone, Copy, Debug)]
pub struct GrepQuery<'a> {
    pub scope: Scope<'a>,
    /// The pattern, as `rg` reads it.
    pub pattern: &'a str,
    /// The pattern as `grep -E` reads it, where the host has no `rg`; `None` when only `rg` can
    /// run it.
    pub extended: Option<&'a str>,
    pub ignore_case: bool,
    /// Matches may span lines, and `.` matches a line break: `rg` only.
    pub multiline: bool,
    pub mode: GrepMode,
}

/// Why a search did not run.
#[derive(Debug)]
pub enum RemoteSearchError {
    /// The path, the pattern or a name is empty or has a line break or NUL.
    InvalidArgument,
    /// The path does not exist.
    NotFound,
    /// The path is a symbolic link whose target does not exist.
    DanglingLink,
    /// `glob`: the path is not a directory: the path, with the directories below it when it is
    /// one, as the script resolved it.
    NotDirectory(String),
    /// The user cannot read the path, or list the directory.
    PermissionDenied,
    /// `rg`, or `grep -E` where the host has no `rg`, refused the pattern: the last line of its
    /// message.
    BadPattern { rg: bool, message: String },
    /// Only `rg` could run the search, and the host has none that knows the options it needs.
    NeedsRg,
    /// The command failed otherwise: its exit status and the last line of its stderr.
    Failed {
        /// `None` when the command ended without one (a signal).
        status: Option<u32>,
        stderr: String,
    },
    /// The channel could not run the command.
    Ssh(SshError),
}

/// What a search printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchOutput {
    /// Where `find` started, as the script resolved it: the directory with `/.` after it, or the
    /// file. Every path printed starts with it.
    pub start: String,
    /// `grep`: `rg` ran, rather than `grep -E`.
    pub rg: bool,
    /// `grep`: each line printed of a file follows the file and a NUL, rather than `:` or `-`.
    pub nul: bool,
    /// What `find`, `rg` or `grep` printed, at most the bytes asked for.
    pub stdout: Vec<u8>,
    /// More followed: `stdout` stopped at the bytes asked for.
    pub cut: bool,
    /// What they reported, such as the files they could not read.
    pub stderr: Vec<u8>,
}

/// Lists the files, and the symbolic links to files, in `scope`, and reads at most `max` bytes of
/// the list. The host ends the search after `time_limit`, and within about a second of the
/// channel closing.
pub async fn glob(
    session: &impl Exec,
    scope: &Scope<'_>,
    max: u64,
    time_limit: Duration,
) -> Result<SearchOutput, RemoteSearchError> {
    let (command, stdin) = glob_request(scope, max, time_limit)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteSearchError::Ssh)?;
    parse(&out, max, false)
}

/// Runs `query`, and reads at most `max` bytes of what `rg` or `grep` prints. The host ends the
/// search as [`glob`]'s.
pub async fn grep(
    session: &impl Exec,
    query: &GrepQuery<'_>,
    max: u64,
    time_limit: Duration,
) -> Result<SearchOutput, RemoteSearchError> {
    let (command, stdin) = grep_request(query, max, time_limit)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteSearchError::Ssh)?;
    parse(&out, max, true)
}

/// One line of stdin, without a line break or NUL; not empty unless `empty` allows it.
fn line(value: &str, empty: bool) -> Result<&str, RemoteSearchError> {
    if (value.is_empty() && !empty) || value.contains(['\n', '\r', '\0']) {
        Err(RemoteSearchError::InvalidArgument)
    } else {
        Ok(value)
    }
}

/// The scope's path, directories below and depth, a line each.
fn scope_lines(scope: &Scope<'_>) -> Result<String, RemoteSearchError> {
    let depth = scope
        .max_depth
        .map_or_else(|| "-".to_owned(), |d| d.max(1).to_string());
    Ok(format!(
        "{}\n{}\n{depth}\n",
        line(scope.path, false)?,
        line(scope.below, true)?
    ))
}

/// The scripts' arguments: one byte more than `max`, which tells whether more followed, and the
/// time limit in whole seconds.
fn command_for(script: &str, max: u64, time_limit: Duration) -> String {
    let seconds = time_limit.as_millis().div_ceil(1000).max(1);
    command(
        script,
        Some(&format!("{} {seconds}", max.saturating_add(1))),
    )
}

fn push_names(stdin: &mut String, names: &[String]) -> Result<(), RemoteSearchError> {
    for name in names {
        stdin.push_str(line(name, false)?);
        stdin.push('\n');
    }
    Ok(())
}

/// The command and stdin of a glob search.
fn glob_request(
    scope: &Scope<'_>,
    max: u64,
    time_limit: Duration,
) -> Result<(String, Vec<u8>), RemoteSearchError> {
    let mut stdin = scope_lines(scope)?;
    stdin.push_str(GLOB_LINKS);
    stdin.push('\n');
    stdin.push_str(RUNNER);
    stdin.push('\n');
    push_names(&mut stdin, scope.names)?;
    Ok((
        command_for(GLOB_SCRIPT, max, time_limit),
        stdin.into_bytes(),
    ))
}

/// The command and stdin of a grep search.
fn grep_request(
    query: &GrepQuery<'_>,
    max: u64,
    time_limit: Duration,
) -> Result<(String, Vec<u8>), RemoteSearchError> {
    let (mode, before, after) = match query.mode {
        GrepMode::Files => ("l", 0, 0),
        GrepMode::Count => ("c", 0, 0),
        GrepMode::Lines { before, after } => ("n", before, after),
    };
    let mut stdin = scope_lines(&query.scope)?;
    stdin.push_str(&format!(
        "{mode}\n{}\n{before}\n{after}\n{}\n{GREP_LINKS}\n{RUNNER}\n{}\n{}\n",
        if query.ignore_case { "i" } else { "s" },
        if query.multiline { "u" } else { "-" },
        line(query.pattern, false)?,
        line(query.extended.unwrap_or_default(), true)?,
    ));
    push_names(&mut stdin, query.scope.names)?;
    Ok((
        command_for(GREP_SCRIPT, max, time_limit),
        stdin.into_bytes(),
    ))
}

fn parse(out: &ExecOutput, max: u64, grep: bool) -> Result<SearchOutput, RemoteSearchError> {
    let failed = || RemoteSearchError::Failed {
        status: out.exit_status,
        stderr: last_line(&out.stderr),
    };
    match out.exit_status {
        // Status 0 without the marker is a failure: a login shell whose startup files exit with 0
        // never ran the script.
        Some(0) => {
            let (start, mut rest) = after_marker(&out.stdout).ok_or_else(failed)?;
            let mut matcher: &[u8] = b"";
            if grep {
                let end = rest.iter().position(|&b| b == b'\n').ok_or_else(failed)?;
                (matcher, rest) = (&rest[..end], &rest[end + 1..]);
            }
            let mut stdout = rest.to_vec();
            let cut = stdout.len() as u64 > max;
            stdout.truncate(usize::try_from(max).unwrap_or(usize::MAX));
            Ok(SearchOutput {
                start,
                rg: matcher.starts_with(b"rg"),
                nul: matcher.ends_with(b" --null"),
                stdout,
                cut,
                stderr: out.stderr.clone(),
            })
        }
        Some(2) => Err(RemoteSearchError::NotFound),
        Some(4) => Err(RemoteSearchError::PermissionDenied),
        Some(6) => Err(RemoteSearchError::NotDirectory(last_line(&out.stderr))),
        Some(8) => Err(RemoteSearchError::DanglingLink),
        Some(status @ (10 | 12)) => Err(RemoteSearchError::BadPattern {
            rg: status == 12,
            message: last_line(&out.stderr),
        }),
        Some(11) => Err(RemoteSearchError::NeedsRg),
        _ => Err(failed()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: Duration = Duration::from_secs(30);

    fn scope<'a>(path: &'a str, names: &'a [String]) -> Scope<'a> {
        Scope {
            path,
            below: "",
            max_depth: None,
            names,
        }
    }

    fn query<'a>(path: &'a str, pattern: &'a str, names: &'a [String]) -> GrepQuery<'a> {
        GrepQuery {
            scope: scope(path, names),
            pattern,
            extended: Some(pattern),
            ignore_case: false,
            multiline: false,
            mode: GrepMode::Files,
        }
    }

    #[test]
    fn scripts_survive_any_login_shell() {
        for script in [GLOB_SCRIPT, GREP_SCRIPT] {
            // `!` too: csh expands it even inside single quotes when its history is on.
            assert!(!script.contains(['\'', '\\', '\n', '!']), "{script}");
        }
        // The helpers travel on stdin, a line each.
        assert!(
            ![GLOB_LINKS, GREP_LINKS, RUNNER]
                .iter()
                .any(|h| h.contains('\n'))
        );
        let names = ["*.conf".to_owned(), "it's [!a]*".to_owned()];
        let mut s = scope("~/etc", &names);
        s.max_depth = Some(2);
        let (command, stdin) = glob_request(&s, 10, Duration::from_millis(1500)).unwrap();
        assert!(command.ends_with("' sh 11 2"), "{command}");
        assert_eq!(
            stdin,
            format!("~/etc\n\n2\n{GLOB_LINKS}\n{RUNNER}\n*.conf\nit's [!a]*\n").as_bytes()
        );
        let mut s = scope("/", &[]);
        s.below = "etc/nginx";
        let (_, stdin) = glob_request(&s, 10, LIMIT).unwrap();
        assert!(stdin.starts_with(b"/\netc/nginx\n-\n"));

        let mut q = query("/etc", r"listen\s+80$", &names[..1]);
        q.extended = Some("listen[[:space:]]+80$");
        q.mode = GrepMode::Lines {
            before: 2,
            after: 0,
        };
        q.ignore_case = true;
        let (_, stdin) = grep_request(&q, 10, LIMIT).unwrap();
        assert_eq!(
            stdin,
            format!(
                "/etc\n\n-\nn\ni\n2\n0\n-\n{GREP_LINKS}\n{RUNNER}\nlisten\\s+80$\nlisten[[:space:]]+80$\n\
                 *.conf\n"
            )
            .as_bytes()
        );
        q.mode = GrepMode::Count;
        q.ignore_case = false;
        q.multiline = true;
        q.extended = None;
        q.scope.max_depth = Some(0);
        let (_, stdin) = grep_request(&q, 10, LIMIT).unwrap();
        assert_eq!(
            stdin,
            format!("/etc\n\n1\nc\ns\n0\n0\nu\n{GREP_LINKS}\n{RUNNER}\nlisten\\s+80$\n\n*.conf\n")
                .as_bytes()
        );

        let mut below = query("/etc", "x", &[]);
        below.scope.below = "a\nb";
        for bad in [
            query("", "x", &[]),
            query("/etc", "", &[]),
            query("/etc", "a\nb", &[]),
            query("a\rb", "x", &[]),
            below,
        ] {
            assert!(matches!(
                grep_request(&bad, 10, LIMIT),
                Err(RemoteSearchError::InvalidArgument)
            ));
        }
        assert!(matches!(
            glob_request(&scope("/", &["a\0".to_owned()]), 10, LIMIT),
            Err(RemoteSearchError::InvalidArgument)
        ));
    }

    #[test]
    fn output_is_read_after_the_marker_and_capped() {
        let out = |stdout: &[u8], status| ExecOutput {
            exit_status: Some(status),
            stdout: stdout.to_vec(),
            stderr: b"find: '/x/./y': Permission denied\n\n\ngrep: Unmatched ( or \\(\n\n".to_vec(),
        };
        let found = parse(
            &out(b"motd\nhatoba-file\n/x/.\n/x/./a\n/x/./b\n", 0),
            100,
            false,
        )
        .unwrap();
        assert_eq!(found.start, "/x/.");
        assert_eq!(
            (&found.stdout[..], found.cut, found.rg, found.nul),
            (&b"/x/./a\n/x/./b\n"[..], false, false, false)
        );
        assert!(found.stderr.starts_with(b"find:"));
        let found = parse(&out(b"hatoba-file\n/x/.\n/x/./a\n/x/./b\n", 0), 9, false).unwrap();
        assert_eq!((&found.stdout[..], found.cut), (&b"/x/./a\n/x"[..], true));
        // grep's output starts with the matcher.
        let found = parse(&out(b"hatoba-file\n/etc/hosts\nrg --null\n", 0), 9, true).unwrap();
        assert_eq!(
            (
                found.start.as_str(),
                found.stdout.len(),
                found.rg,
                found.nul
            ),
            ("/etc/hosts", 0, true, true)
        );
        let found = parse(&out(b"hatoba-file\n/x/.\ngrep\n/x/./a\n", 0), 9, true).unwrap();
        assert_eq!(
            (&found.stdout[..], found.rg, found.nul),
            (&b"/x/./a\n"[..], false, false)
        );

        for (stdout, status) in [(&b""[..], 0), (b"hatoba-file\n/x/.\n", 0), (b"", 1)] {
            assert!(matches!(
                parse(&out(stdout, status), 9, status == 0),
                Err(RemoteSearchError::Failed { .. })
            ));
        }
        assert!(matches!(
            parse(&out(b"", 2), 9, false),
            Err(RemoteSearchError::NotFound)
        ));
        assert!(matches!(
            parse(&out(b"", 6), 9, false),
            Err(RemoteSearchError::NotDirectory(_))
        ));
        assert!(matches!(
            parse(&out(b"", 11), 9, true),
            Err(RemoteSearchError::NeedsRg)
        ));
        // The last line of the message, after the watchdog's empty ones.
        for (status, rg) in [(10, false), (12, true)] {
            match parse(&out(b"", status), 9, true) {
                Err(RemoteSearchError::BadPattern { rg: by, message }) => {
                    assert_eq!((by, message.as_str()), (rg, "grep: Unmatched ( or \\("));
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// The scripts against the local `sh`, `find`, `grep` and, where installed, `rg`, as
    /// [`crate::test_sh`] starts them.
    #[cfg(unix)]
    mod local_sh {
        use std::io::{Read, Write};
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;
        use std::path::Path;
        use std::process::{Command, Stdio};
        use std::time::Instant;

        use super::super::*;
        use super::LIMIT;
        use crate::test_sh::{is_root, run_with};

        /// What the scripts find on `PATH`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Tools {
            /// The host's, with `rg` hidden.
            Grep,
            /// The host's, with `rg` hidden and a `find` without `-xtype`, as on macOS and BusyBox.
            GrepNoXtype,
            /// The host's, `rg` included.
            Rg,
        }

        /// The `PATH` for `tools`: a directory of commands in front of the host's that hide `rg`,
        /// and `-xtype` from `find`, as asked.
        fn path_for(dir: &Path, tools: Tools) -> String {
            let bin = dir.join(format!("bin-{tools:?}"));
            std::fs::create_dir_all(&bin).unwrap();
            let host = std::env::var("PATH").unwrap();
            let script = |name: &str, body: &str| {
                let file = bin.join(name);
                std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            };
            if tools != Tools::Rg {
                script("rg", "exit 127");
            }
            if tools == Tools::GrepNoXtype {
                let find = which("find").unwrap();
                script(
                    "find",
                    &format!(
                        "for a do [ \"$a\" = -xtype ] && {{ echo \"find: unknown primary -xtype\" \
                         >&2; exit 1; }}; done\nexec {find} \"$@\""
                    ),
                );
            }
            format!("{}:{host}", bin.display())
        }

        fn which(command: &str) -> Option<String> {
            let out = Command::new("sh")
                .arg("-c")
                .arg(format!("command -v {command}"))
                .output()
                .unwrap();
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        }

        /// The tools to try: `rg` too where the host has it.
        fn every_tools() -> Vec<Tools> {
            let mut tools = vec![Tools::Grep, Tools::GrepNoXtype];
            if which("rg").is_some() {
                tools.push(Tools::Rg);
            } else {
                eprintln!("rg is not installed: its searches are not tried");
            }
            tools
        }

        /// The lines printed, sorted, with where `find` started taken off.
        fn listed(found: &SearchOutput) -> Vec<String> {
            let mut lines: Vec<String> = String::from_utf8_lossy(&found.stdout)
                .lines()
                .map(|l| l.replacen(&found.start, "", 1))
                .collect();
            lines.sort();
            lines
        }

        fn glob(
            home: &Path,
            path: &str,
            path_arg: &str,
            depth: Option<u32>,
            names: &[&str],
        ) -> Result<SearchOutput, RemoteSearchError> {
            glob_below(home, path, path_arg, "", depth, names)
        }

        fn glob_below(
            home: &Path,
            path: &str,
            path_arg: &str,
            below: &str,
            max_depth: Option<u32>,
            names: &[&str],
        ) -> Result<SearchOutput, RemoteSearchError> {
            let names: Vec<String> = names.iter().map(|&n| n.to_owned()).collect();
            let scope = Scope {
                path: path_arg,
                below,
                max_depth,
                names: &names,
            };
            let (command, stdin) = glob_request(&scope, 1 << 20, LIMIT)?;
            parse(
                &run_with(home, &command, &stdin, &[("PATH", path)]),
                1 << 20,
                false,
            )
        }

        /// A grep search with `PATH` set to `path`.
        fn grep_with(
            home: &Path,
            path: &str,
            query: &GrepQuery<'_>,
            vars: &[(&str, &str)],
        ) -> Result<SearchOutput, RemoteSearchError> {
            let (command, stdin) = grep_request(query, 1 << 20, LIMIT)?;
            let mut all = vec![("PATH", path)];
            all.extend_from_slice(vars);
            parse(&run_with(home, &command, &stdin, &all), 1 << 20, true)
        }

        fn grep(
            home: &Path,
            path: &str,
            at: &str,
            pattern: &str,
            mode: GrepMode,
            names: &[&str],
        ) -> Result<SearchOutput, RemoteSearchError> {
            let names: Vec<String> = names.iter().map(|&n| n.to_owned()).collect();
            let query = GrepQuery {
                scope: Scope {
                    path: at,
                    below: "",
                    max_depth: None,
                    names: &names,
                },
                pattern,
                extended: Some(pattern),
                ignore_case: false,
                multiline: false,
                mode,
            };
            grep_with(home, path, &query, &[])
        }

        /// A tree like an nginx configuration, with names a login shell would mangle.
        fn tree(home: &Path) {
            let etc = home.join("etc");
            for dir in ["conf.d", "sites-available", "sites-enabled", ".git/objects"] {
                std::fs::create_dir_all(etc.join(dir)).unwrap();
            }
            std::fs::write(etc.join("nginx.conf"), "include conf.d/*.conf;\n").unwrap();
            std::fs::write(
                etc.join("conf.d/it's $HOME.conf"),
                "server {\n    listen 8080;\n}\n",
            )
            .unwrap();
            std::fs::write(
                etc.join("sites-available/api"),
                "server {\n  listen 80;\n  proxy_pass http://127.0.0.1:8080;\n}\n",
            )
            .unwrap();
            std::os::unix::fs::symlink("../sites-available/api", etc.join("sites-enabled/api"))
                .unwrap();
            std::fs::write(etc.join(".git/objects/blob"), "listen 8080\n").unwrap();
            std::fs::write(etc.join("binary.conf"), b"listen 8080\0\xff\n").unwrap();
        }

        #[test]
        fn glob_lists_files_and_links_by_name_and_depth() {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            tree(home);
            for tools in [Tools::Grep, Tools::GrepNoXtype] {
                let path = path_for(home, tools);
                let found = glob(home, &path, "etc", None, &["*.conf"]).unwrap();
                assert_eq!(found.start, format!("{}/etc/.", home.display()));
                assert_eq!(
                    listed(&found),
                    ["/binary.conf", "/conf.d/it's $HOME.conf", "/nginx.conf"],
                    "{tools:?}"
                );
                // Any name, links included, .git left out; a depth of 1 stays at the top.
                let all = glob(home, &path, "~/etc/", None, &[]).unwrap();
                assert_eq!(listed(&all).len(), 5, "{tools:?} {:?}", listed(&all));
                assert!(listed(&all).contains(&"/sites-enabled/api".to_owned()));
                assert_eq!(
                    listed(&glob(home, &path, "etc", Some(1), &["*.conf", "*api*"]).unwrap()),
                    ["/binary.conf", "/nginx.conf"]
                );
                // The directories below the path, when it is one.
                let found = glob_below(home, &path, "etc", "sites-enabled", None, &[]).unwrap();
                assert_eq!(
                    (listed(&found).len(), found.start),
                    (1, format!("{}/etc/sites-enabled/.", home.display()))
                );
                assert!(matches!(
                    glob_below(home, &path, "etc/nginx.conf", "conf.d", None, &[]),
                    Err(RemoteSearchError::NotDirectory(_))
                ));
                assert!(matches!(
                    glob_below(home, &path, "etc", "nowhere", None, &[]),
                    Err(RemoteSearchError::NotFound)
                ));
            }
            let path = path_for(home, Tools::GrepNoXtype);
            // Through a link to a directory; links to directories and to nothing are not files.
            std::os::unix::fs::symlink(home.join("etc"), home.join("etc-link")).unwrap();
            assert_eq!(
                listed(&glob(home, &path, "etc-link", Some(1), &["n*"]).unwrap()),
                ["/nginx.conf"]
            );
            std::os::unix::fs::symlink("conf.d", home.join("etc/conf.link")).unwrap();
            std::os::unix::fs::symlink("gone", home.join("etc/gone.conf")).unwrap();
            for tools in [Tools::Grep, Tools::GrepNoXtype] {
                let path = path_for(home, tools);
                assert_eq!(
                    listed(&glob(home, &path, "etc", Some(1), &[]).unwrap()),
                    ["/binary.conf", "/nginx.conf"]
                );
            }

            assert!(matches!(
                glob(home, &path, "missing", None, &[]),
                Err(RemoteSearchError::NotFound)
            ));
            assert!(matches!(
                glob(home, &path, "etc/nginx.conf", None, &[]),
                Err(RemoteSearchError::NotDirectory(_))
            ));
            std::os::unix::fs::symlink(home.join("gone"), home.join("dangling")).unwrap();
            assert!(matches!(
                glob(home, &path, "dangling", None, &[]),
                Err(RemoteSearchError::DanglingLink)
            ));
            if !is_root() {
                let locked = home.join("etc/locked");
                std::fs::create_dir(&locked).unwrap();
                std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
                let found = glob(home, &path, "etc", None, &["*.conf"]).unwrap();
                assert!(
                    String::from_utf8_lossy(&found.stderr).contains("Permission denied"),
                    "{found:?}"
                );
                assert!(matches!(
                    glob(home, &path, "etc/locked", None, &[]),
                    Err(RemoteSearchError::PermissionDenied)
                ));
                std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
        }

        #[test]
        fn grep_finds_files_counts_and_lines() {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            tree(home);
            // A link to a FIFO, which grep would wait on, is not searched.
            let fifo = home.join("etc/fifo");
            assert!(
                Command::new("mkfifo")
                    .arg(&fifo)
                    .status()
                    .unwrap()
                    .success()
            );
            std::os::unix::fs::symlink(&fifo, home.join("etc/fifo.conf")).unwrap();
            for tools in every_tools() {
                let path = path_for(home, tools);
                // Binary files and .git are skipped; a link to a file is searched.
                let files = grep(
                    home,
                    &path,
                    "etc",
                    "listen[[:space:]]+80",
                    GrepMode::Files,
                    &[],
                )
                .unwrap();
                assert_eq!(files.rg, tools == Tools::Rg);
                assert_eq!(
                    listed(&files),
                    [
                        "/conf.d/it's $HOME.conf",
                        "/sites-available/api",
                        "/sites-enabled/api"
                    ],
                    "{tools:?}"
                );
                let counts = grep(home, &path, "etc", "8080", GrepMode::Count, &["api"]).unwrap();
                assert_eq!(
                    listed(&counts),
                    ["/sites-available/api:1", "/sites-enabled/api:1"],
                    "{tools:?}"
                );
                let lines = grep(
                    home,
                    &path,
                    "etc/sites-available/api",
                    "^ *listen",
                    GrepMode::Lines {
                        before: 1,
                        after: 1,
                    },
                    &["ignored"],
                )
                .unwrap();
                assert_eq!(
                    lines.start,
                    format!("{}/etc/sites-available/api", home.display())
                );
                let (m, c) = if lines.nul { ("\0", "\0") } else { (":", "-") };
                assert_eq!(
                    String::from_utf8_lossy(&lines.stdout),
                    format!(
                        "{0}{c}1-server {{\n{0}{m}2:  listen 80;\n{0}{c}3-  proxy_pass \
                         http://127.0.0.1:8080;\n",
                        lines.start
                    ),
                    "{tools:?}"
                );
                // A pattern starting with - is a pattern, and case is ignored when asked.
                let names: Vec<String> = Vec::new();
                let mut query = GrepQuery {
                    scope: Scope {
                        path: "etc",
                        below: "",
                        max_depth: Some(1),
                        names: &names,
                    },
                    pattern: "-* *INCLUDE",
                    extended: Some("-* *INCLUDE"),
                    ignore_case: true,
                    multiline: false,
                    mode: GrepMode::Files,
                };
                let found = grep_with(home, &path, &query, &[]).unwrap();
                assert_eq!(listed(&found), ["/nginx.conf"], "{tools:?}");
                // A file given as the path is searched, whatever directories follow it.
                query.scope.path = "etc/nginx.conf";
                query.scope.below = "conf.d";
                let found = grep_with(home, &path, &query, &[]).unwrap();
                assert_eq!(
                    found.start,
                    format!("{}/etc/nginx.conf", home.display()),
                    "{tools:?}"
                );
                assert_eq!(found.stdout.len(), found.start.len() + 1, "{tools:?}");
                query.scope.path = "etc";
                query.pattern = "8080";
                query.extended = Some("8080");
                query.ignore_case = false;
                query.scope.max_depth = None;
                let found = grep_with(home, &path, &query, &[]).unwrap();
                assert_eq!(
                    (found.start.as_str(), listed(&found)),
                    (
                        format!("{}/etc/conf.d/.", home.display()).as_str(),
                        vec!["/it's $HOME.conf".to_owned()]
                    ),
                    "{tools:?}"
                );

                match grep(home, &path, "etc", "(", GrepMode::Files, &[]) {
                    Err(RemoteSearchError::BadPattern { rg, message }) => {
                        assert_eq!(rg, tools == Tools::Rg);
                        assert!(!message.is_empty());
                    }
                    other => panic!("{tools:?} {other:?}"),
                }
                assert!(matches!(
                    grep(home, &path, "nowhere", "x", GrepMode::Files, &[]),
                    Err(RemoteSearchError::NotFound)
                ));
            }
        }

        #[test]
        fn grep_marks_where_names_end_and_spans_lines_with_rg() {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            let notes = home.join("notes");
            std::fs::create_dir(&notes).unwrap();
            std::fs::write(notes.join("a"), "x\ny\n").unwrap();
            std::fs::write(notes.join("a-2"), "x\n").unwrap();
            std::fs::write(notes.join("b:2:c"), "x\n").unwrap();
            for tools in every_tools() {
                let path = path_for(home, tools);
                let found = grep(
                    home,
                    &path,
                    "notes",
                    "x",
                    GrepMode::Lines {
                        before: 0,
                        after: 1,
                    },
                    &[],
                )
                .unwrap();
                let text = String::from_utf8_lossy(&found.stdout);
                let start = &found.start;
                if found.nul {
                    for line in [
                        format!("{start}/a\x001:x\n{start}/a\x002-y\n"),
                        format!("{start}/a-2\x001:x\n"),
                        format!("{start}/b:2:c\x001:x\n"),
                    ] {
                        assert!(text.contains(&line), "{tools:?} {text:?}");
                    }
                } else {
                    assert!(text.contains(&format!("{start}/b:2:c:1:x\n")), "{text:?}");
                }
                let names: Vec<String> = Vec::new();
                let query = GrepQuery {
                    scope: Scope {
                        path: "notes/a",
                        below: "",
                        max_depth: None,
                        names: &names,
                    },
                    pattern: r"x\ny",
                    extended: None,
                    ignore_case: false,
                    multiline: true,
                    mode: GrepMode::Lines {
                        before: 0,
                        after: 0,
                    },
                };
                match grep_with(home, &path, &query, &[]) {
                    Ok(found) => {
                        assert_eq!(tools, Tools::Rg);
                        let a = found.start;
                        assert_eq!(
                            String::from_utf8_lossy(&found.stdout),
                            format!("{a}\x001:x\n{a}\x002:y\n")
                        );
                    }
                    Err(RemoteSearchError::NeedsRg) => assert_ne!(tools, Tools::Rg),
                    Err(other) => panic!("{tools:?} {other:?}"),
                }
            }
        }

        #[test]
        fn scripts_run_where_lc_all_is_set() {
            // The scripts move LC_ALL aside so that LC_MESSAGES=C applies, and search as before.
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            tree(home);
            let vars = [("LC_ALL", "C")];
            let names = ["*.conf".to_owned()];
            for tools in every_tools() {
                let path = path_for(home, tools);
                let scope = Scope {
                    path: "etc",
                    below: "",
                    max_depth: Some(1),
                    names: &names,
                };
                let (command, stdin) = glob_request(&scope, 1 << 20, LIMIT).unwrap();
                let all = [("PATH", path.as_str()), vars[0]];
                let found = parse(&run_with(home, &command, &stdin, &all), 1 << 20, false).unwrap();
                assert_eq!(listed(&found), ["/binary.conf", "/nginx.conf"]);
                let mut query = GrepQuery {
                    scope: Scope {
                        max_depth: None,
                        ..scope
                    },
                    pattern: "listen[[:space:]]+8080",
                    extended: Some("listen[[:space:]]+8080"),
                    ignore_case: false,
                    multiline: false,
                    mode: GrepMode::Files,
                };
                let found = grep_with(home, &path, &query, &vars).unwrap();
                assert_eq!(listed(&found), ["/conf.d/it's $HOME.conf"], "{tools:?}");
                query.pattern = "(";
                query.extended = Some("(");
                assert!(matches!(
                    grep_with(home, &path, &query, &vars),
                    Err(RemoteSearchError::BadPattern { .. })
                ));
            }
        }

        /// A `find` that never ends, with a child that never ends either, as `find -exec rg` has
        /// one, and records both process IDs; it answers the `-xtype` check. Without `group`,
        /// `setsid` and `perl` fail, so the search cannot have a process group of its own.
        fn endless_find(home: &Path, group: bool) -> String {
            let bin = home.join("bin-endless");
            std::fs::create_dir_all(&bin).unwrap();
            let script = |name: &str, body: &str| {
                let file = bin.join(name);
                std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            };
            script(
                "find",
                "case \" $* \" in *\" -maxdepth 0 \"*) exit 0;; esac\n\
                 sleep 60 < /dev/null > /dev/null 2>&1 &\n\
                 echo \"$$ $!\" > \"$HOME/find.pid\"\nexec sleep 60",
            );
            if !group {
                script("setsid", "exit 127");
                script("perl", "exit 127");
            }
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())
        }

        /// Starts the glob script with the endless `find`, and waits for that to start. The
        /// script shares its process group with the shell that starts it, as with Dropbear, which
        /// gives a command no process group of its own; that shell writes `alive` once the
        /// script is done.
        fn start_endless(
            home: &Path,
            group: bool,
            limit: Duration,
        ) -> (std::process::Child, String) {
            std::fs::create_dir_all(home.join("etc")).unwrap();
            let (command, stdin) = glob_request(
                &Scope {
                    path: "etc",
                    below: "",
                    max_depth: None,
                    names: &[],
                },
                1 << 20,
                limit,
            )
            .unwrap();
            let mut child = Command::new("sh")
                .arg("-c")
                .arg(format!("{command}; echo alive > \"$HOME/alive\""))
                .current_dir("/")
                .env("HOME", home)
                .env("PATH", endless_find(home, group))
                .process_group(0)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&stdin).unwrap();
            let pid_file = home.join("find.pid");
            let started = Instant::now();
            while !pid_file.exists() || std::fs::read_to_string(&pid_file).unwrap().is_empty() {
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "find never started"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            let pid = std::fs::read_to_string(pid_file).unwrap().trim().to_owned();
            (child, pid)
        }

        /// Waits for the script to end, then checks that its `find` and the child of that, `pids`,
        /// ended with it, and that the shell that started it, in the same process group, lived on.
        fn ends_with_find(home: &Path, mut child: std::process::Child, pids: &str) {
            let started = Instant::now();
            while child.try_wait().unwrap().is_none() {
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "the script is still running"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(
                home.join("alive").exists(),
                "the script signalled its own process group"
            );
            // Until its parent's parent reaps it, a process killed is still there.
            let started = Instant::now();
            for pid in pids.split_whitespace() {
                while Command::new("kill")
                    .args(["-0", pid])
                    .stderr(Stdio::null())
                    .status()
                    .unwrap()
                    .success()
                {
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "{pid} of {pids} is still running"
                    );
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }

        #[test]
        fn a_search_ends_when_its_channel_closes() {
            for group in [true, false] {
                let dir = tempfile::tempdir().unwrap();
                let (mut child, pid) = start_endless(dir.path(), group, LIMIT);
                let mut marker = [0; 12];
                child
                    .stdout
                    .as_mut()
                    .unwrap()
                    .read_exact(&mut marker)
                    .unwrap();
                assert_eq!(&marker, b"hatoba-file\n");
                // As sshd does when the channel closes: nothing reads the command's output any
                // more.
                drop(child.stdout.take());
                drop(child.stderr.take());
                ends_with_find(dir.path(), child, &pid);
            }
        }

        #[test]
        fn a_search_ends_at_its_time_limit() {
            for group in [true, false] {
                let dir = tempfile::tempdir().unwrap();
                let (child, pid) = start_endless(dir.path(), group, Duration::from_secs(1));
                ends_with_find(dir.path(), child, &pid);
            }
        }
    }
}

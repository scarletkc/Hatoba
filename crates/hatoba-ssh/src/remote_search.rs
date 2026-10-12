//! Searching the host's files over an exec channel (spec §13.4, "Search tools"): the `find` and
//! `grep` that the AI assistant's `glob` and `grep` run (AI-48, AI-49).
//!
//! Like the file tools' scripts ([`crate::remote_file`]), each is a fixed `sh -c` script whose
//! arguments travel on stdin, one per line, so none goes through the quoting of the user's login
//! shell. They use POSIX `find` and `grep -E` with `-maxdepth` and `head -c`, which GNU, BSD and
//! BusyBox share, and `grep -I` where the host's grep has it. A directory is searched from
//! `dir/.`, so a symbolic link to one is searched too, and every path printed starts with that.
//! Below it, only files and symbolic links to files are listed and searched, checked with GNU
//! `find -xtype f`, or `test -f` where `find` lacks it: links to directories are not followed, and
//! a FIFO, which grep would wait on, is left out. `.git` directories are skipped, and so are
//! `/proc`, `/sys`, `/dev` and `/run` when the search starts at `/`. Messages are in English
//! (`LC_MESSAGES=C`, with an `LC_ALL` moved to the character and collation categories, where it
//! would override that), so the caller can count the files that could not be read.

use crate::error::SshError;
use crate::remote_file::{after_marker, command, last_line};
use crate::session::{ExecOutput, SshSession};

/// Lists the files, and the symbolic links to files, below the directory named on stdin's first
/// line, resolved as the file tools resolve a path, at most `$2` levels down unless stdin's second
/// line is `-`, whose names match one of the `find -name` patterns on the lines after (any name
/// without one). Prints the marker, the directory with `/.` after it, and at most `$1` bytes of
/// `find`'s output. Exit statuses: 2 missing, 4 not readable, 6 not a directory, 8 a symbolic link
/// to nothing, 9 no path.
const GLOB_SCRIPT: &str = "c=$1; IFS= read -r f || exit 9; IFS= read -r d || exit 9; \
     case $f in \"~\") f=$HOME;; \"~/\"*) f=$HOME/${f#\"~/\"};; /*) ;; *) f=$HOME/$f;; esac; \
     [ -L \"$f\" ] && { [ -e \"$f\" ] || exit 8; }; [ -e \"$f\" ] || exit 2; \
     [ -d \"$f\" ] || exit 6; [ -r \"$f\" ] && [ -x \"$f\" ] || exit 4; \
     case $f in */) r=$f.;; *) r=$f/.;; esac; set --; while IFS= read -r n; do \
     if [ $# -eq 0 ]; then set -- -name \"$n\"; else set -- \"$@\" -o -name \"$n\"; fi; done; \
     [ $# -eq 0 ] || set -- \"(\" \"$@\" \")\"; \
     if find \"$r\" -maxdepth 0 -xtype f > /dev/null 2>&1; then set -- \"$@\" -xtype f; \
     else set -- \"$@\" \"(\" -type f -o -type l -exec test -f {} \";\" \")\"; fi; \
     set -- \"$@\" -print; [ \"$r\" = /. ] && set -- \"(\" -path /./proc -o -path /./sys \
     -o -path /./dev -o -path /./run \")\" -prune -o \"$@\"; set -- -name .git -prune -o \"$@\"; \
     [ \"$d\" = - ] || set -- -maxdepth \"$d\" \"$@\"; [ -z \"$LC_ALL\" ] || { \
     LC_CTYPE=$LC_ALL; LC_COLLATE=$LC_ALL; export LC_CTYPE LC_COLLATE; unset LC_ALL; }; \
     LC_MESSAGES=C; export LC_MESSAGES; \
     echo hatoba-file; printf %s \"$r\"; echo; find \"$r\" \"$@\" | head -c \"$c\"";

/// Searches the file, or the files below the directory, named on stdin's first line, as the glob
/// script lists them, with `grep -E` for the pattern on stdin's sixth line. Stdin's second line is
/// the depth (`-` for any), its third the output (`l` the files, `c` the counts of those with a
/// match, `n` the lines), its fourth `i` to ignore case, and its fifth the lines of context; the
/// `find -name` patterns follow the pattern, and a file is searched whatever its name. Prints the
/// marker, where `find` started (the directory with `/.` after it, or the file) and at most `$1`
/// bytes of `grep`'s output. Exit statuses as for the glob script, without 6, and 10 when grep
/// refuses the pattern, with its message on stderr.
const GREP_SCRIPT: &str = "c=$1; IFS= read -r f || exit 9; IFS= read -r d || exit 9; \
     IFS= read -r m || exit 9; IFS= read -r i || exit 9; IFS= read -r x || exit 9; \
     IFS= read -r p || exit 9; \
     case $f in \"~\") f=$HOME;; \"~/\"*) f=$HOME/${f#\"~/\"};; /*) ;; *) f=$HOME/$f;; esac; \
     [ -L \"$f\" ] && { [ -e \"$f\" ] || exit 8; }; [ -e \"$f\" ] || exit 2; \
     [ -r \"$f\" ] || exit 4; [ -z \"$LC_ALL\" ] || { LC_CTYPE=$LC_ALL; LC_COLLATE=$LC_ALL; \
     export LC_CTYPE LC_COLLATE; unset LC_ALL; }; LC_MESSAGES=C; export LC_MESSAGES; \
     : | grep -E -e \"$p\" > /dev/null; [ $? -lt 2 ] || exit 10; o=-EH; \
     [ \"$i\" = i ] && o=\"$o -i\"; case $m in l) o=\"$o -l\";; c) o=\"$o -c\";; \
     *) o=\"$o -n\"; [ \"$x\" = 0 ] || o=\"$o -C $x\";; esac; \
     grep -I -e x /dev/null 2> /dev/null; [ $? -eq 1 ] && o=\"$o -I\"; set --; r=$f; \
     if [ -d \"$f\" ]; then [ -x \"$f\" ] || exit 4; case $f in */) r=$f.;; *) r=$f/.;; esac; \
     while IFS= read -r n; do \
     if [ $# -eq 0 ]; then set -- -name \"$n\"; else set -- \"$@\" -o -name \"$n\"; fi; done; \
     [ $# -eq 0 ] || set -- \"(\" \"$@\" \")\"; fi; \
     if find \"$r\" -maxdepth 0 -xtype f > /dev/null 2>&1; then set -- \"$@\" -xtype f; \
     else set -- \"$@\" \"(\" -type f -o -type l -exec test -f {} \";\" \")\"; fi; \
     if [ -d \"$f\" ]; then [ \"$r\" = /. ] && set -- \"(\" -path /./proc -o -path /./sys \
     -o -path /./dev -o -path /./run \")\" -prune -o \"$@\"; set -- -name .git -prune -o \"$@\"; \
     [ \"$d\" = - ] || set -- -maxdepth \"$d\" \"$@\"; fi; \
     echo hatoba-file; printf %s \"$r\"; echo; case $m in \
     c) find \"$r\" \"$@\" -exec grep $o -e \"$p\" {} + | grep -v -e \":0$\" | head -c \"$c\";; \
     *) find \"$r\" \"$@\" -exec grep $o -e \"$p\" {} + | head -c \"$c\";; esac";

/// What `grep` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrepMode {
    /// The files with a matching line (`grep -l`).
    Files,
    /// The number of matching lines of each file with one (`grep -c`).
    Count,
    /// The matching lines, numbered, with this many lines of context around each.
    Lines { context: u32 },
}

/// A search with `grep`.
#[derive(Clone, Copy, Debug)]
pub struct GrepQuery<'a> {
    /// The file or directory to search, as the model gave it.
    pub path: &'a str,
    /// How many levels below the directory to search; any without.
    pub max_depth: Option<u32>,
    /// `find -name` patterns, one of which each file's name matches; any name without one.
    pub names: &'a [String],
    /// The pattern, as `grep -E` reads it.
    pub pattern: &'a str,
    pub ignore_case: bool,
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
    /// `glob`: the path is not a directory.
    NotDirectory,
    /// The user cannot read the path, or list the directory.
    PermissionDenied,
    /// `grep` refused the pattern: the last line of its message.
    BadPattern(String),
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
    /// What `find` or `grep` printed, at most the bytes asked for.
    pub stdout: Vec<u8>,
    /// More followed: `stdout` stopped at the bytes asked for.
    pub cut: bool,
    /// What `find` and `grep` reported, such as the files they could not read.
    pub stderr: Vec<u8>,
}

/// Lists the files, and the symbolic links to files, below the directory `path` whose names match
/// one of `names` (any name when empty), at most `max_depth` levels down, and reads at most `max`
/// bytes of the list.
pub async fn glob(
    session: &SshSession,
    path: &str,
    max_depth: Option<u32>,
    names: &[String],
    max: u64,
) -> Result<SearchOutput, RemoteSearchError> {
    let (command, stdin) = glob_request(path, max_depth, names, max)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteSearchError::Ssh)?;
    parse(&out, max)
}

/// Runs `query`, and reads at most `max` bytes of what `grep` prints.
pub async fn grep(
    session: &SshSession,
    query: &GrepQuery<'_>,
    max: u64,
) -> Result<SearchOutput, RemoteSearchError> {
    let (command, stdin) = grep_request(query, max)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteSearchError::Ssh)?;
    parse(&out, max)
}

/// One line of stdin: not empty, without a line break or NUL.
fn line(value: &str) -> Result<&str, RemoteSearchError> {
    if value.is_empty() || value.contains(['\n', '\r', '\0']) {
        Err(RemoteSearchError::InvalidArgument)
    } else {
        Ok(value)
    }
}

fn depth(max_depth: Option<u32>) -> String {
    max_depth.map_or_else(|| "-".to_owned(), |d| d.max(1).to_string())
}

/// The command and stdin of a glob search. One byte more than `max` is asked for, which tells
/// whether more followed.
fn glob_request(
    path: &str,
    max_depth: Option<u32>,
    names: &[String],
    max: u64,
) -> Result<(String, Vec<u8>), RemoteSearchError> {
    let mut stdin = format!("{}\n{}\n", line(path)?, depth(max_depth));
    for name in names {
        stdin.push_str(line(name)?);
        stdin.push('\n');
    }
    Ok((
        command(GLOB_SCRIPT, Some(&max.saturating_add(1).to_string())),
        stdin.into_bytes(),
    ))
}

/// The command and stdin of a grep search, as for [`glob_request`].
fn grep_request(query: &GrepQuery<'_>, max: u64) -> Result<(String, Vec<u8>), RemoteSearchError> {
    let (mode, context) = match query.mode {
        GrepMode::Files => ("l", 0),
        GrepMode::Count => ("c", 0),
        GrepMode::Lines { context } => ("n", context),
    };
    let mut stdin = format!(
        "{}\n{}\n{mode}\n{}\n{context}\n{}\n",
        line(query.path)?,
        depth(query.max_depth),
        if query.ignore_case { "i" } else { "s" },
        line(query.pattern)?,
    );
    for name in query.names {
        stdin.push_str(line(name)?);
        stdin.push('\n');
    }
    Ok((
        command(GREP_SCRIPT, Some(&max.saturating_add(1).to_string())),
        stdin.into_bytes(),
    ))
}

fn parse(out: &ExecOutput, max: u64) -> Result<SearchOutput, RemoteSearchError> {
    let failed = || RemoteSearchError::Failed {
        status: out.exit_status,
        stderr: last_line(&out.stderr),
    };
    match out.exit_status {
        // Status 0 without the marker is a failure: a login shell whose startup files exit with 0
        // never ran the script.
        Some(0) => {
            let (start, rest) = after_marker(&out.stdout).ok_or_else(failed)?;
            let mut stdout = rest.to_vec();
            let cut = stdout.len() as u64 > max;
            stdout.truncate(usize::try_from(max).unwrap_or(usize::MAX));
            Ok(SearchOutput {
                start,
                stdout,
                cut,
                stderr: out.stderr.clone(),
            })
        }
        Some(2) => Err(RemoteSearchError::NotFound),
        Some(4) => Err(RemoteSearchError::PermissionDenied),
        Some(6) => Err(RemoteSearchError::NotDirectory),
        Some(8) => Err(RemoteSearchError::DanglingLink),
        Some(10) => Err(RemoteSearchError::BadPattern(last_line(&out.stderr))),
        _ => Err(failed()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query<'a>(path: &'a str, pattern: &'a str, names: &'a [String]) -> GrepQuery<'a> {
        GrepQuery {
            path,
            max_depth: None,
            names,
            pattern,
            ignore_case: false,
            mode: GrepMode::Files,
        }
    }

    #[test]
    fn scripts_survive_any_login_shell() {
        for script in [GLOB_SCRIPT, GREP_SCRIPT] {
            // `!` too: csh expands it even inside single quotes when its history is on.
            assert!(!script.contains(['\'', '\\', '\n', '!']), "{script}");
        }
        let names = ["*.conf".to_owned(), "it's [!a]*".to_owned()];
        let (command, stdin) = glob_request("~/etc", Some(2), &names, 10).unwrap();
        assert!(command.ends_with("' sh 11"), "{command}");
        assert_eq!(stdin, b"~/etc\n2\n*.conf\nit's [!a]*\n");
        let (_, stdin) = glob_request("/", None, &[], 10).unwrap();
        assert_eq!(stdin, b"/\n-\n");

        let mut q = query("/etc", "listen[[:space:]]+80$", &names[..1]);
        q.mode = GrepMode::Lines { context: 2 };
        q.ignore_case = true;
        let (_, stdin) = grep_request(&q, 10).unwrap();
        assert_eq!(stdin, b"/etc\n-\nn\ni\n2\nlisten[[:space:]]+80$\n*.conf\n");
        q.mode = GrepMode::Count;
        q.ignore_case = false;
        q.max_depth = Some(0);
        let (_, stdin) = grep_request(&q, 10).unwrap();
        assert_eq!(stdin, b"/etc\n1\nc\ns\n0\nlisten[[:space:]]+80$\n*.conf\n");

        for bad in [
            query("", "x", &[]),
            query("/etc", "", &[]),
            query("/etc", "a\nb", &[]),
            query("a\rb", "x", &[]),
        ] {
            assert!(matches!(
                grep_request(&bad, 10),
                Err(RemoteSearchError::InvalidArgument)
            ));
        }
        assert!(matches!(
            glob_request("/", None, &["a\0".to_owned()], 10),
            Err(RemoteSearchError::InvalidArgument)
        ));
    }

    #[test]
    fn output_is_read_after_the_marker_and_capped() {
        let out = |stdout: &[u8], status| ExecOutput {
            exit_status: Some(status),
            stdout: stdout.to_vec(),
            stderr: b"find: '/x/./y': Permission denied\ngrep: Unmatched ( or \\(\n".to_vec(),
        };
        let found = parse(&out(b"motd\nhatoba-file\n/x/.\n/x/./a\n/x/./b\n", 0), 100).unwrap();
        assert_eq!(found.start, "/x/.");
        assert_eq!(
            (&found.stdout[..], found.cut),
            (&b"/x/./a\n/x/./b\n"[..], false)
        );
        assert!(found.stderr.starts_with(b"find:"));
        let found = parse(&out(b"hatoba-file\n/x/.\n/x/./a\n/x/./b\n", 0), 9).unwrap();
        assert_eq!((&found.stdout[..], found.cut), (&b"/x/./a\n/x"[..], true));
        let found = parse(&out(b"hatoba-file\n/etc/hosts\n", 0), 9).unwrap();
        assert_eq!(
            (found.start.as_str(), found.stdout.len()),
            ("/etc/hosts", 0)
        );

        assert!(matches!(
            parse(&out(b"", 0), 9),
            Err(RemoteSearchError::Failed { .. })
        ));
        assert!(matches!(
            parse(&out(b"", 2), 9),
            Err(RemoteSearchError::NotFound)
        ));
        assert!(matches!(
            parse(&out(b"", 6), 9),
            Err(RemoteSearchError::NotDirectory)
        ));
        match parse(&out(b"", 10), 9) {
            Err(RemoteSearchError::BadPattern(message)) => {
                assert_eq!(message, "grep: Unmatched ( or \\(");
            }
            other => panic!("{other:?}"),
        }
    }

    /// The scripts against the local `sh`, `find` and `grep` (GNU on Linux, BSD on macOS), started
    /// the way a login shell would start them, in `/`.
    #[cfg(unix)]
    mod local_sh {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;
        use std::process::{Command, Stdio};

        use super::super::*;

        fn run(home: &Path, command: &str, stdin: &[u8]) -> ExecOutput {
            run_with(home, command, stdin, &[])
        }

        /// [`run`] with these variables set as well.
        fn run_with(home: &Path, command: &str, stdin: &[u8], vars: &[(&str, &str)]) -> ExecOutput {
            let mut child = Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir("/")
                .env("HOME", home)
                .envs(vars.iter().copied())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(stdin).unwrap();
            let out = child.wait_with_output().unwrap();
            ExecOutput {
                exit_status: out.status.code().map(|c| c as u32),
                stdout: out.stdout,
                stderr: out.stderr,
            }
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
            depth: Option<u32>,
            names: &[&str],
        ) -> Result<SearchOutput, RemoteSearchError> {
            let names: Vec<String> = names.iter().map(|&n| n.to_owned()).collect();
            let (command, stdin) = glob_request(path, depth, &names, 1 << 20)?;
            parse(&run(home, &command, &stdin), 1 << 20)
        }

        fn grep(
            home: &Path,
            path: &str,
            pattern: &str,
            mode: GrepMode,
            names: &[&str],
        ) -> Result<SearchOutput, RemoteSearchError> {
            let names: Vec<String> = names.iter().map(|&n| n.to_owned()).collect();
            let query = GrepQuery {
                path,
                max_depth: None,
                names: &names,
                pattern,
                ignore_case: false,
                mode,
            };
            let (command, stdin) = grep_request(&query, 1 << 20)?;
            parse(&run(home, &command, &stdin), 1 << 20)
        }

        fn is_root() -> bool {
            Command::new("id")
                .arg("-u")
                .output()
                .is_ok_and(|o| o.stdout.starts_with(b"0\n"))
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
            let found = glob(home, "etc", None, &["*.conf"]).unwrap();
            assert_eq!(found.start, format!("{}/etc/.", home.display()));
            assert_eq!(
                listed(&found),
                ["/binary.conf", "/conf.d/it's $HOME.conf", "/nginx.conf"]
            );
            // Any name, links included, .git left out; a depth of 1 stays at the top.
            let all = glob(home, "~/etc/", None, &[]).unwrap();
            assert_eq!(listed(&all).len(), 5, "{:?}", listed(&all));
            assert!(listed(&all).contains(&"/sites-enabled/api".to_owned()));
            assert_eq!(
                listed(&glob(home, "etc", Some(1), &["*.conf", "*api*"]).unwrap()),
                ["/binary.conf", "/nginx.conf"]
            );
            // Through a link to a directory; links to directories and to nothing are not files.
            std::os::unix::fs::symlink(home.join("etc"), home.join("etc-link")).unwrap();
            assert_eq!(
                listed(&glob(home, "etc-link", Some(1), &["n*"]).unwrap()),
                ["/nginx.conf"]
            );
            std::os::unix::fs::symlink("conf.d", home.join("etc/conf.link")).unwrap();
            std::os::unix::fs::symlink("gone", home.join("etc/gone.conf")).unwrap();
            assert_eq!(
                listed(&glob(home, "etc", Some(1), &[]).unwrap()),
                ["/binary.conf", "/nginx.conf"]
            );

            assert!(matches!(
                glob(home, "missing", None, &[]),
                Err(RemoteSearchError::NotFound)
            ));
            assert!(matches!(
                glob(home, "etc/nginx.conf", None, &[]),
                Err(RemoteSearchError::NotDirectory)
            ));
            std::os::unix::fs::symlink(home.join("gone"), home.join("dangling")).unwrap();
            assert!(matches!(
                glob(home, "dangling", None, &[]),
                Err(RemoteSearchError::DanglingLink)
            ));
            if !is_root() {
                let locked = home.join("etc/locked");
                std::fs::create_dir(&locked).unwrap();
                std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
                let found = glob(home, "etc", None, &["*.conf"]).unwrap();
                assert!(
                    String::from_utf8_lossy(&found.stderr).contains("Permission denied"),
                    "{found:?}"
                );
                assert!(matches!(
                    glob(home, "etc/locked", None, &[]),
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
            // Binary files and .git are skipped; a link is followed, but not one to a FIFO, which
            // grep would wait on.
            let fifo = home.join("etc/fifo");
            assert!(
                Command::new("mkfifo")
                    .arg(&fifo)
                    .status()
                    .unwrap()
                    .success()
            );
            std::os::unix::fs::symlink(&fifo, home.join("etc/fifo.conf")).unwrap();
            let files = grep(home, "etc", "listen[[:space:]]+80", GrepMode::Files, &[]).unwrap();
            assert_eq!(
                listed(&files),
                [
                    "/conf.d/it's $HOME.conf",
                    "/sites-available/api",
                    "/sites-enabled/api"
                ]
            );
            let counts = grep(home, "etc", "8080", GrepMode::Count, &["api"]).unwrap();
            assert_eq!(
                listed(&counts),
                ["/sites-available/api:1", "/sites-enabled/api:1"]
            );
            let lines = grep(
                home,
                "etc/sites-available/api",
                "^ *listen",
                GrepMode::Lines { context: 1 },
                &["ignored"],
            )
            .unwrap();
            assert_eq!(
                lines.start,
                format!("{}/etc/sites-available/api", home.display())
            );
            assert_eq!(
                String::from_utf8_lossy(&lines.stdout),
                format!(
                    "{0}-1-server {{\n{0}:2:  listen 80;\n{0}-3-  proxy_pass \
                     http://127.0.0.1:8080;\n",
                    lines.start
                )
            );
            // A pattern starting with - is a pattern, and case is ignored when asked.
            let names: Vec<String> = Vec::new();
            let query = GrepQuery {
                path: "etc",
                max_depth: Some(1),
                names: &names,
                pattern: "-* *INCLUDE",
                ignore_case: true,
                mode: GrepMode::Files,
            };
            let (command, stdin) = grep_request(&query, 1 << 20).unwrap();
            let found = parse(&run(home, &command, &stdin), 1 << 20).unwrap();
            assert_eq!(listed(&found), ["/nginx.conf"]);

            match grep(home, "etc", "(", GrepMode::Files, &[]) {
                Err(RemoteSearchError::BadPattern(message)) => assert!(!message.is_empty()),
                other => panic!("{other:?}"),
            }
            assert!(matches!(
                grep(home, "nowhere", "x", GrepMode::Files, &[]),
                Err(RemoteSearchError::NotFound)
            ));
        }

        #[test]
        fn scripts_run_where_lc_all_is_set() {
            // The scripts move LC_ALL aside so that LC_MESSAGES=C applies, and search as before.
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            tree(home);
            let vars = [("LC_ALL", "C")];
            let names = ["*.conf".to_owned()];
            let (command, stdin) = glob_request("etc", Some(1), &names, 1 << 20).unwrap();
            let found = parse(&run_with(home, &command, &stdin, &vars), 1 << 20).unwrap();
            assert_eq!(listed(&found), ["/binary.conf", "/nginx.conf"]);
            let mut query = GrepQuery {
                path: "etc",
                max_depth: None,
                names: &names,
                pattern: "listen[[:space:]]+8080",
                ignore_case: false,
                mode: GrepMode::Files,
            };
            let (command, stdin) = grep_request(&query, 1 << 20).unwrap();
            let found = parse(&run_with(home, &command, &stdin, &vars), 1 << 20).unwrap();
            assert_eq!(listed(&found), ["/conf.d/it's $HOME.conf"]);
            query.pattern = "(";
            let (command, stdin) = grep_request(&query, 1 << 20).unwrap();
            assert!(matches!(
                parse(&run_with(home, &command, &stdin, &vars), 1 << 20),
                Err(RemoteSearchError::BadPattern(_))
            ));
        }
    }
}

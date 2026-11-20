use super::glob::{expand_braces, fnmatch, host_name};
use super::pattern::{
    ASCII_CLASS, CLASS_OPERATION, CONTROL, FLAGS, MULTILINE, NEGATED_IN_CLASS, TEXT_ANCHOR,
    UNICODE_CLASS,
};
use super::results::{Root, problems};
use super::*;
use crate::tools::{GrepArgs, GrepOutput};

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
    let stdout =
        format!("/s/./a:1:x\r\n/s/./a-2-after\n/s/./b-4-before\n/s/./b:5:{long}\n/s/./c\0:1:bin\n");
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

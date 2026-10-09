//! `~/.ssh/config` import (SSH-11).
//!
//! Only the options Hatoba can map onto a host entry are understood: `Host`,
//! `HostName`, `User`, `Port`, `IdentityFile`, `ProxyJump` and `SetEnv`.
//! `ProxyCommand` is read only so the import can say it was left out. Lookup
//! follows OpenSSH semantics: blocks are evaluated in file order and the first
//! value obtained for an option wins (`IdentityFile` accumulates, and the first
//! `SetEnv` line that applies wins as a whole).
//! `Match` blocks and `Include` are ignored.

use std::path::PathBuf;

use serde::Serialize;

/// One concrete host alias from an SSH config file with all matching
/// blocks (including `Host *` defaults) already applied.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct SshConfigHost {
    /// The alias used on the `Host` line.
    pub alias: String,
    /// `HostName` (real address); `None` means the alias itself is the address.
    pub hostname: Option<String>,
    /// `User`.
    pub user: Option<String>,
    /// `Port`.
    pub port: Option<u16>,
    /// `IdentityFile` entries in order of precedence, `~` and `%d` expanded.
    pub identity_files: Vec<String>,
    /// Raw `ProxyJump` value, e.g. `user@bastion:2222,other`.
    pub proxy_jump: Option<String>,
    /// Raw `ProxyCommand` value, e.g. `nc -X 5 -x proxy:1080 %h %p`. Never set together with
    /// `proxy_jump`.
    pub proxy_command: Option<String>,
    /// `SetEnv` variables as name and value, in order. Names are not checked;
    /// arguments without `=` are dropped.
    pub set_env: Vec<(String, String)>,
}

#[derive(Debug, Default)]
struct Block {
    /// Patterns of the `Host` line; empty for the implicit global block.
    patterns: Vec<String>,
    /// `Match` blocks are parsed but never applied.
    ignored: bool,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity_files: Vec<String>,
    /// `ProxyJump` and `ProxyCommand` lines in file order, because which one applies depends on
    /// their order (see [`resolve_proxy`]).
    proxy: Vec<ProxyDirective>,
    set_env: Option<Vec<(String, String)>>,
}

#[derive(Debug)]
enum ProxyDirective {
    Jump(String),
    Command(String),
}

/// Parses SSH config text, expanding `~` / `%d` using the current user's home
/// directory (`HOME`, or `USERPROFILE` on Windows).
pub fn parse_ssh_config(text: &str) -> Vec<SshConfigHost> {
    parse_ssh_config_with_home(text, home_dir().as_deref())
}

/// Like [`parse_ssh_config`] with an explicit home directory (for tests and
/// for importing a config file that belongs to another profile).
pub fn parse_ssh_config_with_home(text: &str, home: Option<&str>) -> Vec<SshConfigHost> {
    let blocks = parse_blocks(text);

    // Concrete aliases in order of first appearance.
    let mut aliases: Vec<String> = Vec::new();
    for block in blocks.iter().filter(|b| !b.ignored) {
        for p in &block.patterns {
            if !is_wildcard_or_negated(p) && !aliases.iter().any(|a| a == p) {
                aliases.push(p.clone());
            }
        }
    }

    aliases
        .into_iter()
        .map(|alias| resolve_alias(&alias, &blocks, home))
        .collect()
}

fn home_dir() -> Option<String> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .and_then(|p| p.to_str().map(str::to_owned))
        .filter(|s| !s.is_empty())
}

fn is_wildcard_or_negated(pattern: &str) -> bool {
    pattern.starts_with('!') || pattern.contains(['*', '?'])
}

fn parse_blocks(text: &str) -> Vec<Block> {
    let mut blocks = vec![Block::default()];
    for raw in text.lines() {
        let Some((keyword, args)) = split_directive(raw) else {
            continue;
        };
        match keyword.to_ascii_lowercase().as_str() {
            "host" => blocks.push(Block {
                ignored: args.is_empty(),
                patterns: args,
                ..Block::default()
            }),
            "match" => blocks.push(Block {
                ignored: true,
                ..Block::default()
            }),
            kw => {
                let Some(block) = blocks.last_mut() else {
                    continue;
                };
                if kw == "proxycommand" {
                    if !args.is_empty() {
                        block.proxy.push(ProxyDirective::Command(args.join(" ")));
                    }
                    continue;
                }
                if kw == "setenv" {
                    if block.set_env.is_none() {
                        block.set_env = parse_set_env(args);
                    }
                    continue;
                }
                let first = args.into_iter().next();
                match (kw, first) {
                    ("hostname", Some(v)) => {
                        block.hostname.get_or_insert(v);
                    }
                    ("user", Some(v)) => {
                        block.user.get_or_insert(v);
                    }
                    ("port", Some(v)) => {
                        if block.port.is_none() {
                            block.port = v.parse().ok();
                        }
                    }
                    ("identityfile", Some(v)) => block.identity_files.push(v),
                    ("proxyjump", Some(v)) => {
                        block.proxy.push(ProxyDirective::Jump(v));
                    }
                    _ => {}
                }
            }
        }
    }
    blocks
}

/// `ProxyJump` and `ProxyCommand` as OpenSSH reads them (`readconf.c`, `parse_jump`): the first
/// `ProxyCommand` applies; a `ProxyJump` to a host applies only before any `ProxyCommand` or other
/// `ProxyJump`, and then sets the command to `none`, which blocks later ones; `ProxyJump none`
/// takes the jump slot without blocking a later `ProxyCommand`; `none` clears.
fn resolve_proxy<'a>(
    directives: impl Iterator<Item = &'a ProxyDirective>,
) -> (Option<String>, Option<String>) {
    let mut jump: Option<&str> = None;
    let mut command: Option<&str> = None;
    for directive in directives {
        match directive {
            ProxyDirective::Command(v) => {
                command.get_or_insert(v);
            }
            ProxyDirective::Jump(v) if v.eq_ignore_ascii_case("none") => {
                jump.get_or_insert(v);
            }
            ProxyDirective::Jump(v) => {
                if command.is_none() && jump.is_none() {
                    jump = Some(v);
                    command = Some("none");
                }
            }
        }
    }
    let set = |v: Option<&str>| {
        v.filter(|v| !v.eq_ignore_ascii_case("none"))
            .map(str::to_owned)
    };
    (set(jump), set(command))
}

/// The `NAME=value` arguments of one `SetEnv` line; `None` when it has none.
/// A name given twice keeps its first value, as in OpenSSH.
fn parse_set_env(args: Vec<String>) -> Option<Vec<(String, String)>> {
    let mut vars: Vec<(String, String)> = Vec::new();
    for arg in args {
        let Some((name, value)) = arg.split_once('=') else {
            continue;
        };
        if !vars.iter().any(|(n, _)| n == name) {
            vars.push((name.to_owned(), value.to_owned()));
        }
    }
    (!vars.is_empty()).then_some(vars)
}

fn resolve_alias(alias: &str, blocks: &[Block], home: Option<&str>) -> SshConfigHost {
    let mut host = SshConfigHost {
        alias: alias.to_owned(),
        ..SshConfigHost::default()
    };
    let mut env_set = false;
    let applying: Vec<&Block> = blocks
        .iter()
        .filter(|b| !b.ignored && block_applies(b, alias))
        .collect();
    (host.proxy_jump, host.proxy_command) =
        resolve_proxy(applying.iter().flat_map(|b| b.proxy.iter()));

    for block in applying {
        if host.hostname.is_none() {
            host.hostname = block.hostname.clone();
        }
        if host.user.is_none() {
            host.user = block.user.clone();
        }
        if host.port.is_none() {
            host.port = block.port;
        }
        host.identity_files
            .extend(block.identity_files.iter().cloned());
        if !env_set && let Some(vars) = &block.set_env {
            env_set = true;
            host.set_env = vars.clone();
        }
    }

    // `%h` inside HostName itself refers to the alias typed by the user.
    let mut ctx = ExpandCtx {
        alias,
        hostname: alias.to_owned(),
        user: host.user.clone(),
        port: host.port.unwrap_or(22),
        home,
    };
    host.hostname = host.hostname.map(|h| expand_tokens(&h, &ctx));
    // Everything else sees the final HostName.
    ctx.hostname = host.hostname.clone().unwrap_or_else(|| alias.to_owned());
    host.identity_files = host
        .identity_files
        .iter()
        .filter(|f| !f.eq_ignore_ascii_case("none"))
        .map(|f| expand_path(f, &ctx))
        .collect();
    host
}

fn block_applies(block: &Block, alias: &str) -> bool {
    if block.patterns.is_empty() {
        return true; // global section before the first Host line
    }
    let mut matched = false;
    for p in &block.patterns {
        if let Some(negated) = p.strip_prefix('!') {
            if glob_match(negated, alias) {
                return false;
            }
        } else if glob_match(p, alias) {
            matched = true;
        }
    }
    matched
}

/// Case-insensitive `*` / `?` glob match.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

struct ExpandCtx<'a> {
    alias: &'a str,
    hostname: String,
    user: Option<String>,
    port: u16,
    home: Option<&'a str>,
}

/// Expands `%h %n %p %r %d %%`; unknown tokens are kept verbatim.
fn expand_tokens(value: &str, ctx: &ExpandCtx<'_>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('h') => out.push_str(&ctx.hostname),
            Some('n') => out.push_str(ctx.alias),
            Some('p') => out.push_str(&ctx.port.to_string()),
            Some('r') => match &ctx.user {
                Some(u) => out.push_str(u),
                None => out.push_str("%r"),
            },
            Some('d') => match ctx.home {
                Some(h) => out.push_str(h),
                None => out.push_str("%d"),
            },
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn expand_path(value: &str, ctx: &ExpandCtx<'_>) -> String {
    let expanded = expand_tokens(value, ctx);
    match (expanded.strip_prefix('~'), ctx.home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            format!("{home}{rest}")
        }
        _ => expanded,
    }
}

/// Splits a config line into its keyword and arguments. Returns `None` for
/// blank lines and comments.
fn split_directive(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let key_end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let keyword = &line[..key_end];
    let mut rest = line[key_end..].trim_start();
    if let Some(r) = rest.strip_prefix('=') {
        rest = r.trim_start();
    }
    Some((keyword.to_owned(), tokenize(rest)))
}

/// Whitespace-separated arguments, split as OpenSSH's `argv_split` does:
/// `"double"` and `'single'` quotes group words, and a backslash escapes a
/// quote or a backslash anywhere and a space outside quotes. Any other
/// backslash is kept, so Windows paths survive. An unquoted token starting
/// with `#` begins a comment.
fn tokenize(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        match chars.peek() {
            None => break,
            Some('#') => break,
            _ => {}
        }
        let mut token = String::new();
        let mut quote: Option<char> = None;
        while let Some(&c) = chars.peek() {
            if c == '\\' {
                chars.next();
                let unquoted = quote.is_none();
                let escaped =
                    chars.next_if(|n| matches!(n, '"' | '\'' | '\\') || (*n == ' ' && unquoted));
                token.push(escaped.unwrap_or('\\'));
            } else if quote == Some(c) {
                quote = None;
                chars.next();
            } else if quote.is_none() && matches!(c, '"' | '\'') {
                quote = Some(c);
                chars.next();
            } else if c.is_whitespace() && quote.is_none() {
                break;
            } else {
                token.push(c);
                chars.next();
            }
        }
        tokens.push(token);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<SshConfigHost> {
        parse_ssh_config_with_home(text, Some("/home/me"))
    }

    #[test]
    fn basic_hosts() {
        let hosts = parse(
            "Host prod\n  HostName 10.0.0.5\n  User deploy\n  Port 2222\n  IdentityFile ~/.ssh/prod_ed25519\n\nHost dev\n  HostName dev.example.com\n",
        );
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].alias, "prod");
        assert_eq!(hosts[0].hostname.as_deref(), Some("10.0.0.5"));
        assert_eq!(hosts[0].user.as_deref(), Some("deploy"));
        assert_eq!(hosts[0].port, Some(2222));
        assert_eq!(hosts[0].identity_files, vec!["/home/me/.ssh/prod_ed25519"]);
        assert_eq!(hosts[1].alias, "dev");
        assert_eq!(hosts[1].user, None);
        assert_eq!(hosts[1].port, None);
    }

    #[test]
    fn multiple_aliases_yield_multiple_entries() {
        let hosts = parse("Host a b\n  HostName shared.example.com\n");
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].alias, "a");
        assert_eq!(hosts[1].alias, "b");
        assert_eq!(hosts[1].hostname.as_deref(), Some("shared.example.com"));
    }

    #[test]
    fn wildcard_and_negated_patterns_are_skipped() {
        let hosts = parse(
            "Host *\n  User x\nHost !foo\n  Port 1\nHost *.example.com\n  Port 2\nHost real\n",
        );
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "real");
    }

    #[test]
    fn host_star_defaults_apply_first_value_wins() {
        let hosts = parse(
            "Host web\n  User alice\n  IdentityFile ~/.ssh/web\n\nHost db\n  Port 5432\n\nHost *\n  User default\n  Port 2200\n  IdentityFile ~/.ssh/id_default\n",
        );
        assert_eq!(hosts[0].user.as_deref(), Some("alice"));
        assert_eq!(hosts[0].port, Some(2200));
        assert_eq!(
            hosts[0].identity_files,
            vec!["/home/me/.ssh/web", "/home/me/.ssh/id_default"]
        );
        assert_eq!(hosts[1].user.as_deref(), Some("default"));
        assert_eq!(hosts[1].port, Some(5432));
    }

    #[test]
    fn host_star_first_wins_over_later_specific() {
        // OpenSSH: the first obtained value wins, even if a later block is more specific.
        let hosts = parse("Host *\n  User first\nHost web\n  User second\n");
        assert_eq!(hosts[0].user.as_deref(), Some("first"));
    }

    #[test]
    fn pattern_blocks_apply_to_matching_aliases() {
        let hosts = parse("Host web*\n  User www\nHost web1\n  HostName 1.2.3.4\n");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].user.as_deref(), Some("www"));
        assert_eq!(hosts[0].hostname.as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn negation_excludes_matches() {
        let hosts = parse("Host *.corp !secret.corp\n  User corp\nHost secret.corp other.corp\n");
        let by_alias = |a: &str| hosts.iter().find(|h| h.alias == a).unwrap();
        assert_eq!(by_alias("other.corp").user.as_deref(), Some("corp"));
        assert_eq!(by_alias("secret.corp").user, None);
    }

    #[test]
    fn keywords_case_insensitive_equals_and_quotes() {
        let hosts = parse(
            "host   \"my box\" other\n  hostname=1.1.1.1\n  USER = bob\n  port   \"2022\"\n  IdentityFile \"/path with space/key\"\n",
        );
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].alias, "my box");
        assert_eq!(hosts[0].hostname.as_deref(), Some("1.1.1.1"));
        assert_eq!(hosts[0].user.as_deref(), Some("bob"));
        assert_eq!(hosts[0].port, Some(2022));
        assert_eq!(hosts[0].identity_files, vec!["/path with space/key"]);
    }

    #[test]
    fn comments_are_ignored() {
        let hosts = parse(
            "# top comment\nHost a # trailing comment\n  # indented comment\n  HostName a.example.com # why\n  User u#notacomment\n",
        );
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "a");
        assert_eq!(hosts[0].hostname.as_deref(), Some("a.example.com"));
        assert_eq!(hosts[0].user.as_deref(), Some("u#notacomment"));
    }

    #[test]
    fn proxy_jump_kept_raw_and_none_disables() {
        let hosts = parse(
            "Host target\n  ProxyJump user@bastion:2222,other\nHost direct\n  ProxyJump none\nHost *\n  ProxyJump default-jump\n",
        );
        assert_eq!(
            hosts[0].proxy_jump.as_deref(),
            Some("user@bastion:2222,other")
        );
        assert_eq!(hosts[1].proxy_jump, None);
    }

    #[test]
    fn proxy_command_and_proxy_jump_follow_openssh_order() {
        let hosts = parse(concat!(
            "Host socks\n  ProxyCommand nc -X 5 -x proxy.example.com:1080 %h %p\n",
            "Host jumped\n  ProxyJump bastion\n",
            "Host off\n  ProxyCommand none\n",
            "Host command-first\n  ProxyCommand nc %h %p\n  ProxyJump bastion\n",
            "Host jump-first\n  ProxyJump bastion\n  ProxyCommand nc %h %p\n",
            "Host jump-none\n  ProxyJump none\n  ProxyCommand nc -x proxy:1080 %h %p\n",
            "Host jump-none-only\n  ProxyJump none\n",
            "Host command-none\n  ProxyCommand none\n  ProxyJump bastion\n",
            "Host *\n  ProxyCommand connect -H proxy:3128 %h %p\n",
        ));
        let pair = |i: usize| {
            (
                hosts[i].proxy_jump.as_deref(),
                hosts[i].proxy_command.as_deref(),
            )
        };
        assert_eq!(
            pair(0),
            (None, Some("nc -X 5 -x proxy.example.com:1080 %h %p"))
        );
        // A jump host blocks every later ProxyCommand, the default's included.
        assert_eq!(pair(1), (Some("bastion"), None));
        assert_eq!(pair(2), (None, None));
        assert_eq!(pair(3), (None, Some("nc %h %p")));
        assert_eq!(pair(4), (Some("bastion"), None));
        // `ProxyJump none` does not hide a later ProxyCommand, in the same block or the default.
        assert_eq!(pair(5), (None, Some("nc -x proxy:1080 %h %p")));
        assert_eq!(pair(6), (None, Some("connect -H proxy:3128 %h %p")));
        // `ProxyCommand none` blocks a later ProxyJump and then clears itself.
        assert_eq!(pair(7), (None, None));
    }

    #[test]
    fn set_env_takes_several_quoted_variables() {
        let hosts = parse(
            "Host app\n  SetEnv TZ=Asia/Tokyo GREETING=\"hello world\" 'QUOTED=it''s' EMPTY= NOEQUALS LANG=ja_JP.UTF-8 TZ=UTC\n",
        );
        assert_eq!(
            hosts[0].set_env,
            [
                ("TZ".to_owned(), "Asia/Tokyo".to_owned()),
                ("GREETING".to_owned(), "hello world".to_owned()),
                ("QUOTED".to_owned(), "its".to_owned()),
                ("EMPTY".to_owned(), String::new()),
                ("LANG".to_owned(), "ja_JP.UTF-8".to_owned()),
            ]
        );
        let hosts = parse("Host app\n  SetEnv=A=\"x=y\" # comment\n");
        assert_eq!(hosts[0].set_env, [("A".to_owned(), "x=y".to_owned())]);
    }

    #[test]
    fn first_set_env_line_that_applies_wins_whole() {
        // OpenSSH: the first SetEnv obtained is used as a whole; later lines don't add to it.
        let hosts = parse(
            "Host web\n  SetEnv A=1\n  SetEnv B=2\nHost db\n  SetEnv NOEQUALS\nHost *\n  SetEnv A=star C=3\nHost plain\n",
        );
        let by_alias = |a: &str| &hosts.iter().find(|h| h.alias == a).unwrap().set_env;
        assert_eq!(*by_alias("web"), [("A".to_owned(), "1".to_owned())]);
        // A line without any NAME=value sets nothing, so the defaults apply.
        assert_eq!(
            *by_alias("db"),
            [
                ("A".to_owned(), "star".to_owned()),
                ("C".to_owned(), "3".to_owned())
            ]
        );
        assert_eq!(by_alias("plain").len(), 2);
        assert!(parse("Host none\n").pop().unwrap().set_env.is_empty());
    }

    #[test]
    fn single_quotes_group_words() {
        let hosts = parse("Host 'my box'\n  User \"o'neil\"\n");
        assert_eq!(hosts[0].alias, "my box");
        assert_eq!(hosts[0].user.as_deref(), Some("o'neil"));
    }

    #[test]
    fn unquoted_backslashes_escape_like_openssh() {
        // `\ `, `\"`, `\'` and `\\` are escapes outside quotes; other backslashes stay.
        let hosts = parse(concat!(
            "Host app\n",
            "  SetEnv GREETING=hello\\ world QUOTE=a\\\"b APOS=it\\'s SLASH=a\\\\b TAB=a\\tb\n",
            "  User o\\'neil\n",
            "  IdentityFile C:\\Users\\me\\.ssh\\id_ed25519\n",
        ));
        assert_eq!(
            hosts[0].set_env,
            [
                ("GREETING".to_owned(), "hello world".to_owned()),
                ("QUOTE".to_owned(), "a\"b".to_owned()),
                ("APOS".to_owned(), "it's".to_owned()),
                ("SLASH".to_owned(), "a\\b".to_owned()),
                ("TAB".to_owned(), "a\\tb".to_owned()),
            ]
        );
        assert_eq!(hosts[0].user.as_deref(), Some("o'neil"));
        assert_eq!(hosts[0].identity_files, ["C:\\Users\\me\\.ssh\\id_ed25519"]);
        // Inside quotes a backslash before a space is kept.
        let hosts = parse("Host app\n  SetEnv \"A=x\\ y\"\n");
        assert_eq!(hosts[0].set_env, [("A".to_owned(), "x\\ y".to_owned())]);
    }

    #[test]
    fn token_expansion() {
        let hosts = parse(
            "Host box\n  HostName %h.internal\n  User me\n  IdentityFile %d/.ssh/%r_%h\n  IdentityFile ~\\keys\\k\n",
        );
        assert_eq!(hosts[0].hostname.as_deref(), Some("box.internal"));
        assert_eq!(
            hosts[0].identity_files,
            vec!["/home/me/.ssh/me_box.internal", "/home/me\\keys\\k"]
        );
    }

    #[test]
    fn invalid_port_ignored_and_match_blocks_skipped() {
        let hosts =
            parse("Host a\n  Port notaport\nMatch host a\n  User ignored\nHost b\n  User kept\n");
        assert_eq!(hosts[0].port, None);
        assert_eq!(hosts[0].user, None);
        assert_eq!(hosts[1].user.as_deref(), Some("kept"));
    }

    #[test]
    fn options_before_first_host_are_global_defaults() {
        let hosts = parse("User globaluser\nHost a\n  HostName a.example\n");
        assert_eq!(hosts[0].user.as_deref(), Some("globaluser"));
    }

    #[test]
    fn crlf_and_empty_input() {
        assert!(parse("").is_empty());
        let hosts = parse("Host a\r\n  HostName x\r\n");
        assert_eq!(hosts[0].hostname.as_deref(), Some("x"));
    }

    #[test]
    fn realistic_windows_config() {
        let text = "\
# Hatoba import test
Include config.d/*

Host bastion
    HostName bastion.example.com
    User ops
    Port 2222
    IdentityFile ~/.ssh/bastion_ed25519
    IdentitiesOnly yes

Host app-* !app-legacy
    ProxyJump ops@bastion:2222
    User deploy

Host app-1 app-2 app-legacy
    HostName %h.internal.example.com

Host *
    ServerAliveInterval 30
    IdentityFile ~/.ssh/id_ed25519
    IdentityFile ~/.ssh/id_rsa
";
        let hosts = parse_ssh_config_with_home(text, Some("C:\\Users\\me"));
        let names: Vec<_> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(names, ["bastion", "app-1", "app-2", "app-legacy"]);

        let bastion = &hosts[0];
        assert_eq!(bastion.hostname.as_deref(), Some("bastion.example.com"));
        assert_eq!(bastion.port, Some(2222));
        assert_eq!(
            bastion.identity_files,
            [
                "C:\\Users\\me/.ssh/bastion_ed25519",
                "C:\\Users\\me/.ssh/id_ed25519",
                "C:\\Users\\me/.ssh/id_rsa"
            ]
        );
        let app1 = &hosts[1];
        assert_eq!(app1.hostname.as_deref(), Some("app-1.internal.example.com"));
        assert_eq!(app1.user.as_deref(), Some("deploy"));
        assert_eq!(app1.proxy_jump.as_deref(), Some("ops@bastion:2222"));
        // `!app-legacy` excludes the second block's settings from app-legacy.
        let legacy = &hosts[3];
        assert_eq!(legacy.user, None);
        assert_eq!(legacy.proxy_jump, None);
        assert_eq!(
            legacy.hostname.as_deref(),
            Some("app-legacy.internal.example.com")
        );
    }

    #[test]
    fn glob_matcher() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("web?", "web1"));
        assert!(!glob_match("web?", "web12"));
        assert!(glob_match("*.example.com", "a.b.example.com"));
        assert!(!glob_match("*.example.com", "example.com"));
        assert!(glob_match("A*b", "axxB"));
    }
}

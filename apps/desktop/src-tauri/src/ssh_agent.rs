//! The ssh-agent each hop of a connection signs in with (SSH-09).
//!
//! On macOS and Linux a hop uses the agent `ssh user@host` would use in a terminal: the
//! `IdentityAgent` of `~/.ssh/config` that applies to the hop's address, or else the
//! `SSH_AUTH_SOCK` the login shell exports when it differs from the one Hatoba inherited, with
//! the inherited one after it. A window started from Finder or a desktop launcher never reads
//! `~/.zshrc`, so without the login shell an agent set up there, Bitwarden's for one, would look
//! absent. When neither applies, the hop keeps the agent `SSH_AUTH_SOCK` names, as before.
//!
//! Windows keeps the agent it always had: the OpenSSH pipe, or the pipe `SSH_AUTH_SOCK` names.

use hatoba_ssh::ConnectConfig;

/// Fills the agent sockets of the hops of `cfg` that sign in with the agent. The first call may
/// wait for the login shell, which runs once (see `platform::login_shell`).
pub async fn resolve(cfg: &mut ConnectConfig) {
    use hatoba_ssh::AuthMethod;

    if !cfg!(unix) {
        return;
    }
    let uses_agent =
        |auth: &AuthMethod| matches!(auth, AuthMethod::Agent | AuthMethod::AgentThenAsk);
    // Each hop's address, user and port, which `IdentityAgent` is matched and expanded with.
    let hops: Vec<Option<(String, String, u16)>> = cfg
        .jump
        .iter()
        .map(|j| (&j.auth, &j.host, &j.username, j.port))
        .chain(std::iter::once((
            &cfg.auth,
            &cfg.host,
            &cfg.username,
            cfg.port,
        )))
        .map(|(auth, host, user, port)| {
            uses_agent(auth).then(|| (host.clone(), user.clone(), port))
        })
        .collect();
    if hops.iter().all(Option::is_none) {
        return;
    }
    let Ok(mut sockets) = tauri::async_runtime::spawn_blocking(move || {
        let home = std::env::var("HOME").ok().filter(|h| !h.is_empty());
        let config = home.as_deref().and_then(|h| {
            std::fs::read_to_string(std::path::Path::new(h).join(".ssh/config")).ok()
        });
        let inherited = std::env::var("SSH_AUTH_SOCK")
            .ok()
            .filter(|s| !s.is_empty());
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let shell = || crate::platform::login_shell::ssh_auth_sock().map(str::to_owned);
        hops.into_iter()
            .map(|hop| {
                let Some((host, user, port)) = hop else {
                    return Vec::new();
                };
                let identity_agent = config.as_deref().and_then(|text| {
                    hatoba_ssh::config::identity_agent(text, &host, &user, port, home.as_deref())
                });
                sockets_for(identity_agent.as_deref(), inherited.as_deref(), env, shell)
            })
            .collect::<Vec<_>>()
    })
    .await
    else {
        return;
    };
    cfg.agent_sockets = sockets.pop().unwrap_or_default();
    for (jump, sockets) in cfg.jump.iter_mut().zip(sockets) {
        jump.agent_sockets = sockets;
    }
}

/// The sockets one hop tries, in order; empty keeps the agent `SSH_AUTH_SOCK` names.
///
/// `identity_agent` is the hop's `IdentityAgent` as `~/.ssh/config` writes it, `inherited` the
/// `SSH_AUTH_SOCK` Hatoba started with, `env` reads a variable an `IdentityAgent` of `$NAME`
/// points at, and `shell` the login shell's `SSH_AUTH_SOCK`, asked only when it is needed.
fn sockets_for(
    identity_agent: Option<&str>,
    inherited: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
    shell: impl FnOnce() -> Option<String>,
) -> Vec<String> {
    match identity_agent {
        // `none` turns the agent off for `ssh`, but a host set to sign in with the agent here still
        // wants one, so it gets the one `SSH_AUTH_SOCK` names, as `SSH_AUTH_SOCK` asks outright.
        None | Some("none" | "SSH_AUTH_SOCK" | "$SSH_AUTH_SOCK") => {}
        Some(value) => match value.strip_prefix('$') {
            Some(name) => {
                if let Some(path) = env(name) {
                    return vec![path];
                }
            }
            None => return vec![value.to_owned()],
        },
    }
    match shell() {
        Some(exported) if inherited != Some(exported.as_str()) => std::iter::once(exported)
            .chain(inherited.map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn shell(value: Option<&str>) -> impl FnOnce() -> Option<String> {
        let value = value.map(str::to_owned);
        move || value
    }

    #[test]
    fn identity_agent_is_used_alone_without_asking_the_shell() {
        let sockets = sockets_for(
            Some("/Users/me/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"),
            Some("/var/run/com.apple.launchd.x/Listeners"),
            no_env,
            || panic!("the login shell is not needed"),
        );
        assert_eq!(
            sockets,
            ["/Users/me/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock"]
        );
    }

    #[test]
    fn identity_agent_can_name_a_variable() {
        let env = |name: &str| (name == "BW_SOCK").then(|| "/home/me/bw.sock".to_owned());
        let sockets = sockets_for(Some("$BW_SOCK"), None, env, shell(None));
        assert_eq!(sockets, ["/home/me/bw.sock"]);
        // A variable that is not set leaves the hop to `SSH_AUTH_SOCK`.
        let sockets = sockets_for(Some("$NOT_SET"), None, env, shell(Some("/tmp/a.sock")));
        assert_eq!(sockets, ["/tmp/a.sock"]);
    }

    #[test]
    fn the_shell_value_comes_first_when_it_differs() {
        let sockets = sockets_for(
            None,
            Some("/var/run/com.apple.launchd.x/Listeners"),
            no_env,
            shell(Some("/Users/me/.bitwarden-ssh-agent.sock")),
        );
        assert_eq!(
            sockets,
            [
                "/Users/me/.bitwarden-ssh-agent.sock",
                "/var/run/com.apple.launchd.x/Listeners"
            ]
        );
        let sockets = sockets_for(None, None, no_env, shell(Some("/home/me/bw.sock")));
        assert_eq!(sockets, ["/home/me/bw.sock"]);
    }

    #[test]
    fn the_inherited_agent_stays_when_the_shell_adds_nothing() {
        let inherited = Some("/run/user/1000/keyring/ssh");
        assert!(sockets_for(None, inherited, no_env, shell(inherited)).is_empty());
        assert!(sockets_for(None, inherited, no_env, shell(None)).is_empty());
        assert!(sockets_for(None, None, no_env, shell(None)).is_empty());
        for keyword in ["none", "SSH_AUTH_SOCK", "$SSH_AUTH_SOCK"] {
            assert!(sockets_for(Some(keyword), inherited, no_env, shell(None)).is_empty());
        }
    }
}

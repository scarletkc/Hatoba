//! ssh-agent access (SSH-09).
//!
//! Where an agent listens:
//!
//! * **Windows**: the OpenSSH named pipe `\\.\pipe\openssh-ssh-agent`. The OpenSSH
//!   Authentication Agent service owns it, and the Bitwarden desktop app takes it over once that
//!   service is stopped, which is what Bitwarden's own instructions ask for, so Bitwarden works
//!   there without anything else. `SSH_AUTH_SOCK` is used when it names a pipe.
//! * **macOS and Linux**: the Unix socket `SSH_AUTH_SOCK` names, used alone — it is what the shell
//!   of this session chose. Only when that variable is unset, or holds the placeholder launchd
//!   gives a window started from Finder (which names no agent at all, and which `~/.zshrc` never
//!   gets a chance to overwrite), the well-known agent sockets are searched instead: without them,
//!   an agent the user configured there (Bitwarden's, 1Password's, GnuPG's or the GNOME keyring's)
//!   would look absent to Hatoba.
//!
//! A socket the user picks in Settings → SSH Agent is used alone, so a path that stops working
//! fails with that path named instead of quietly using another agent.

use std::path::{Path, PathBuf};
use std::time::Duration;

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};

use crate::error::SshError;

/// A connected agent, with the stream type erased so the platform paths agree.
pub(crate) type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

/// Name of the Win32-OpenSSH agent pipe.
#[cfg(windows)]
const WINDOWS_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// The socket Bitwarden's desktop app listens on beside the home directory, on every platform
/// where it uses a Unix socket. Named in the error that says no agent was found.
const BITWARDEN_SOCKET: &str = "~/.bitwarden-ssh-agent.sock";

/// How long a socket has to answer with its identities before the next candidate is tried. A
/// socket that accepts connections without being an agent must not hold the connection up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether `path` is the placeholder macOS gives `SSH_AUTH_SOCK` in a window started from Finder.
///
/// It is not an agent: connecting to it asks launchd to start the user's own `ssh-agent`, which
/// holds no keys, and that empty agent would hide the one the user actually uses (Bitwarden's,
/// say). OpenSSH clients are pointed at it by `launchctl`, and `ssh` finds the keys because the
/// agent it starts is the one `ssh-add` fills; Hatoba never adds keys, so it skips it.
fn is_launchd_placeholder(path: &Path) -> bool {
    let path = path.to_string_lossy();
    // Where launchd puts the listener depends on how the process was started: a login session on
    // this macOS answers `/var/run/com.apple.launchd.<id>/Listeners`, and a window started another
    // way can be handed the same listener under `/private/tmp`. Both mean "no agent here".
    [
        "/var/run/com.apple.launchd.",
        "/private/tmp/com.apple.launchd.",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix) && path.ends_with("/Listeners"))
}

/// The agent behind a socket, so the settings page can name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    /// An agent Hatoba does not recognize from the path: the system agent, or one listening on a
    /// socket of its own.
    System,
    Bitwarden,
    OnePassword,
    GnuPgp,
    GnomeKeyring,
}

impl AgentKind {
    /// Which agent a socket path belongs to, as far as its path says.
    fn of(path: &Path) -> Self {
        let path = path.to_string_lossy().to_ascii_lowercase();
        if path.contains("bitwarden") {
            Self::Bitwarden
        } else if path.contains("1password") {
            Self::OnePassword
        } else if path.contains("gpg-agent") {
            Self::GnuPgp
        } else if path.contains("keyring") {
            Self::GnomeKeyring
        } else {
            Self::System
        }
    }
}

/// One place an agent may listen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSocket {
    /// The Unix socket path, or the Windows named pipe.
    pub path: PathBuf,
    /// The agent this socket looks like.
    pub kind: AgentKind,
}

impl AgentSocket {
    fn new(path: PathBuf) -> Self {
        let kind = AgentKind::of(&path);
        Self { path, kind }
    }

    /// The socket as the settings page shows it.
    pub fn display(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

/// What the discovery reads out of the environment, so a test can hand it its own values
/// instead of changing the environment of the whole test binary.
#[derive(Debug, Default)]
struct Env {
    /// `SSH_AUTH_SOCK`: the socket the shell of this session points at.
    socket: Option<PathBuf>,
    /// `HOME`: beside it live the sockets of Bitwarden, 1Password and GnuPG.
    home: Option<PathBuf>,
    /// `XDG_RUNTIME_DIR`: there live the GnuPG and GNOME keyring sockets.
    runtime: Option<PathBuf>,
    /// `GNUPGHOME`: when set, GnuPG keeps its socket there instead of under `HOME`.
    gnupg: Option<PathBuf>,
}

impl Env {
    fn of_process() -> Self {
        let var = |key: &str| {
            std::env::var_os(key)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            socket: var("SSH_AUTH_SOCK"),
            home: var("HOME"),
            runtime: var("XDG_RUNTIME_DIR"),
            gnupg: var("GNUPGHOME"),
        }
    }
}

/// The sockets to try, in order: the user's choice alone when there is one, otherwise the socket
/// `SSH_AUTH_SOCK` names (when it exists) and then the well-known ones. Without duplicates, so a
/// variable that names a known agent's socket is tried once.
pub fn agent_sockets(chosen: Option<&str>) -> Vec<AgentSocket> {
    candidates(chosen, &Env::of_process())
}

/// The path as the user typed it, with a leading `~` read as the home directory: people write
/// `~/.bitwarden-ssh-agent.sock` by hand, and the shell that would have expanded it is not here.
/// `~user` is left alone, as nothing here knows another user's home.
fn expand_home(path: &str, env: &Env) -> PathBuf {
    match env.home.as_deref() {
        Some(home) if path == "~" => home.to_path_buf(),
        Some(home) => match path.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(path),
        },
        None => PathBuf::from(path),
    }
}

fn candidates(chosen: Option<&str>, env: &Env) -> Vec<AgentSocket> {
    if let Some(path) = chosen.map(str::trim).filter(|p| !p.is_empty()) {
        return vec![AgentSocket::new(expand_home(path, env))];
    }
    // A socket this session's environment sets is honoured as it is: using another agent behind it
    // would authenticate with a key the user did not point this session at, and hide a path that
    // stopped working instead of naming it. The launchd placeholder is the one value that is not a
    // choice — macOS writes it into every window started from Finder, and it names no agent — so
    // only there are the sockets below searched.
    #[cfg(unix)]
    if let Some(socket) = env.socket.clone().filter(|p| !is_launchd_placeholder(p)) {
        return vec![AgentSocket::new(socket)];
    }
    // On Windows the variable holds a pipe name when it holds anything usable; anything else there
    // is not an agent either, and the OpenSSH pipe below is.
    #[cfg(windows)]
    if let Some(socket) = env.socket.clone().filter(|p| is_pipe(p)) {
        return vec![AgentSocket::new(socket)];
    }
    let mut sockets: Vec<AgentSocket> = Vec::new();
    // The OpenSSH pipe is always worth a try on Windows: the OpenSSH Authentication Agent
    // service or a Bitwarden desktop app with that service stopped owns it.
    #[cfg(windows)]
    sockets.push(AgentSocket::new(PathBuf::from(WINDOWS_AGENT_PIPE)));
    for socket in known_sockets(env) {
        if !sockets.iter().any(|s| s.path == socket.path) {
            sockets.push(socket);
        }
    }
    sockets
}

/// The paths the discovery guesses when `SSH_AUTH_SOCK` names nothing. Each of them is a socket
/// only while that agent runs, which is why every one of them is checked for existence.
#[cfg(unix)]
fn known_sockets(env: &Env) -> Vec<AgentSocket> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(home) = &env.home {
        paths.extend(
            [
                // Bitwarden: the `.dmg` build of the desktop app, and the same path on Linux.
                ".bitwarden-ssh-agent.sock",
                // The macOS App Store build, which is sandboxed.
                "Library/Containers/com.bitwarden.desktop/Data/.bitwarden-ssh-agent.sock",
                // A Linux snap or Flatpak install.
                "snap/bitwarden/current/.bitwarden-ssh-agent.sock",
                ".var/app/com.bitwarden.desktop/data/.bitwarden-ssh-agent.sock",
                // 1Password: the macOS group container and the Linux agent.
                "Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock",
                ".1password/agent.sock",
                // GnuPG with `enable-ssh-support`, with its home in the default place.
                ".gnupg/S.gpg-agent.ssh",
            ]
            .iter()
            .map(|rel| home.join(rel)),
        );
    }
    if let Some(runtime) = &env.runtime {
        paths.push(runtime.join("gnupg/S.gpg-agent.ssh"));
        // The GNOME keyring's ssh component.
        paths.push(runtime.join("keyring/ssh"));
    }
    if let Some(gnupg) = &env.gnupg {
        paths.push(gnupg.join("S.gpg-agent.ssh"));
    }
    paths
        .into_iter()
        .filter(|p| p.exists())
        .map(AgentSocket::new)
        .collect()
}

/// Windows agents all listen on named pipes, which [`known_sockets`] on Unix covers: the
/// OpenSSH pipe joined the candidates in `candidates`, since it is always worth a try.
#[cfg(windows)]
fn known_sockets(_env: &Env) -> Vec<AgentSocket> {
    Vec::new()
}

/// Whether `path` is a Windows named pipe, the only thing an agent can answer on there.
#[cfg(windows)]
fn is_pipe(path: &Path) -> bool {
    path.to_string_lossy().starts_with(r"\\.\pipe\")
}

/// Connects to the agent to authenticate with, and returns it with the socket it answered on and
/// the identities it holds.
///
/// A candidate counts only when it answers the agent protocol. A socket that accepts a connection
/// without being an agent is left out, and so is the launchd placeholder (see
/// [`is_launchd_placeholder`]). An agent that answers but holds no keys does not stop the search
/// either, so that an empty one macOS starts on demand cannot hide the agent the user uses; when
/// no candidate holds keys, the first that answered is returned, and the caller says so.
pub(crate) async fn connect(
    chosen: Option<&str>,
) -> Result<(Agent, AgentSocket, Vec<AgentIdentity>), SshError> {
    let sockets = agent_sockets(chosen);
    let mut refusals: Vec<String> = Vec::new();
    let mut empty: Option<(Agent, AgentSocket)> = None;
    for socket in &sockets {
        // The connect is bounded as well as the request: a busy Windows named pipe retries
        // inside the client with no deadline of its own, so waiting for it there would keep the
        // whole search pending past the probe timeout.
        let mut agent = match tokio::time::timeout(PROBE_TIMEOUT, open(socket)).await {
            Ok(Ok(agent)) => agent,
            Ok(Err(message)) => {
                tracing::debug!(
                    "ssh-agent at {} did not answer: {message}",
                    socket.display()
                );
                refusals.push(message);
                continue;
            }
            Err(_) => {
                tracing::debug!("ssh-agent at {} did not answer in time", socket.display());
                refusals.push(format!(
                    "{}: no answer in {} seconds",
                    socket.display(),
                    PROBE_TIMEOUT.as_secs()
                ));
                continue;
            }
        };
        match tokio::time::timeout(PROBE_TIMEOUT, agent.request_identities()).await {
            Ok(Ok(identities)) if !identities.is_empty() => {
                tracing::debug!(
                    "ssh-agent at {} holds {} identities",
                    socket.display(),
                    identities.len()
                );
                return Ok((agent, socket.clone(), identities));
            }
            Ok(Ok(_)) => {
                tracing::debug!("ssh-agent at {} holds no identities", socket.display());
                empty.get_or_insert((agent, socket.clone()));
            }
            Ok(Err(e)) => {
                tracing::debug!("{} is not an agent: {e}", socket.display());
                refusals.push(format!("{}: {e}", socket.display()));
            }
            Err(_) => {
                tracing::debug!("ssh-agent at {} did not answer in time", socket.display());
                refusals.push(format!(
                    "{}: no answer in {} seconds",
                    socket.display(),
                    PROBE_TIMEOUT.as_secs()
                ));
            }
        }
    }
    if let Some((agent, socket)) = empty {
        // An agent without keys is the answer when nothing else has any: a caller that
        // authenticates with it reports `ssh-agent holds no identities`, as it always did.
        return Ok((agent, socket, Vec::new()));
    }
    Err(no_agent(&sockets, refusals.last()))
}

/// Opens one socket, with the platform's own connect call. The error is the reason to report.
#[cfg(unix)]
async fn open(socket: &AgentSocket) -> Result<Agent, String> {
    AgentClient::connect_uds(&socket.path)
        .await
        .map(|client| client.dynamic())
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
async fn open(socket: &AgentSocket) -> Result<Agent, String> {
    AgentClient::connect_named_pipe(&socket.display())
        .await
        .map(|client| client.dynamic())
        .map_err(|e| e.to_string())
}

#[cfg(not(any(unix, windows)))]
async fn open(_socket: &AgentSocket) -> Result<Agent, String> {
    Err("ssh-agent is not supported on this platform".to_owned())
}

/// Why no agent was found, naming what was tried.
fn no_agent(sockets: &[AgentSocket], refusal: Option<&String>) -> SshError {
    if sockets.is_empty() {
        let hint = if cfg!(windows) {
            "is the \"OpenSSH Authentication Agent\" service running?"
        } else {
            "set SSH_AUTH_SOCK, or pick a socket in Settings → SSH Agent"
        };
        return SshError::other(format!(
            "no ssh-agent is available: SSH_AUTH_SOCK is not set or names nothing, and no known \
             agent socket exists (Bitwarden's is {BITWARDEN_SOCKET}); {hint}"
        ));
    }
    let tried = sockets
        .iter()
        .map(AgentSocket::display)
        .collect::<Vec<_>>()
        .join(", ");
    match refusal {
        Some(reason) => SshError::other(format!("no ssh-agent answered at {tried}: {reason}")),
        None => SshError::other(format!("no ssh-agent answered at {tried}")),
    }
}

/// One identity an agent holds, for Settings → SSH Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentityInfo {
    /// The key type, such as `ssh-ed25519`.
    pub algorithm: String,
    /// The SHA-256 fingerprint, as OpenSSH writes it (`SHA256:...`).
    pub fingerprint: String,
    /// The identity's comment, empty when it has none.
    pub comment: String,
}

/// What Settings → SSH Agent shows: the socket that answered and the keys it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatus {
    pub socket: AgentSocket,
    pub identities: Vec<AgentIdentityInfo>,
}

/// Lists the identities of the first agent that answers, without authenticating anywhere.
pub async fn agent_status(chosen: Option<&str>) -> Result<AgentStatus, SshError> {
    let (_agent, socket, identities) = connect(chosen).await?;
    Ok(AgentStatus {
        socket,
        identities: identities.iter().map(identity_info).collect(),
    })
}

/// Reads one identity into what the settings page shows.
fn identity_info(identity: &AgentIdentity) -> AgentIdentityInfo {
    match identity {
        AgentIdentity::PublicKey { key, comment } => AgentIdentityInfo {
            algorithm: key.algorithm().as_str().to_owned(),
            fingerprint: crate::keys::fingerprint_sha256(&key.to_openssh().unwrap_or_default()),
            comment: comment.clone(),
        },
        AgentIdentity::Certificate {
            certificate,
            comment,
        } => AgentIdentityInfo {
            algorithm: certificate.algorithm().as_str().to_owned(),
            fingerprint: crate::keys::fingerprint_sha256(
                &certificate.to_openssh().unwrap_or_default(),
            ),
            comment: comment.clone(),
        },
    }
}

/// The discovery on Unix only: Windows agents all answer on named pipes, and `known_sockets`
/// has nothing to guess there, so these assertions would not hold on a Windows build (which
/// `cargo test -p hatoba-ssh` runs in CI).
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A file at `dir/rel`, which stands in for an agent's socket: discovery only checks that it
    /// exists, and connecting is the caller's job.
    fn touch(dir: &Path, rel: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, b"").unwrap();
        path
    }

    /// An environment whose home and runtime directory live in `dir`.
    fn env_at(dir: &Path) -> Env {
        Env {
            home: Some(dir.join("home")),
            runtime: Some(dir.join("run")),
            ..Env::default()
        }
    }

    #[test]
    fn a_chosen_socket_is_the_only_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = touch(dir.path(), "home/bitwarden.sock");
        let sockets = candidates(Some(&chosen.to_string_lossy()), &env_at(dir.path()));
        assert_eq!(sockets.len(), 1);
        assert_eq!(sockets[0].path, chosen);
        assert_eq!(sockets[0].kind, AgentKind::Bitwarden);
    }

    #[cfg(unix)]
    #[test]
    fn both_launchd_listener_paths_are_the_placeholder() {
        for path in [
            // Measured on macOS 27 with `launchctl getenv SSH_AUTH_SOCK`.
            "/var/run/com.apple.launchd.jy4KjsWd0t/Listeners",
            "/private/tmp/com.apple.launchd.abc123/Listeners",
        ] {
            assert!(is_launchd_placeholder(Path::new(path)), "{path}");
        }
        assert!(!is_launchd_placeholder(Path::new(
            "/Users/someone/.bitwarden-ssh-agent.sock"
        )));
        assert!(!is_launchd_placeholder(Path::new(
            "/var/run/com.apple.launchd.x/Other"
        )));
    }

    #[test]
    fn a_leading_tilde_in_a_chosen_path_is_the_home_directory() {
        let env = Env {
            socket: None,
            home: Some(PathBuf::from("/Users/someone")),
            runtime: None,
            gnupg: None,
        };
        assert_eq!(
            expand_home("~/.bitwarden-ssh-agent.sock", &env),
            PathBuf::from("/Users/someone/.bitwarden-ssh-agent.sock")
        );
        assert_eq!(expand_home("~", &env), PathBuf::from("/Users/someone"));
        // Another user's home is not ours to guess, and an absolute path is left alone.
        assert_eq!(
            expand_home("~someone/agent.sock", &env),
            PathBuf::from("~someone/agent.sock")
        );
        assert_eq!(
            expand_home("/run/agent.sock", &env),
            PathBuf::from("/run/agent.sock")
        );
    }

    #[test]
    fn a_socket_this_session_set_is_the_only_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let bitwarden = touch(dir.path(), "home/.bitwarden-ssh-agent.sock");
        let env = Env {
            socket: Some(touch(dir.path(), "home/agent.sock")),
            ..env_at(dir.path())
        };

        // The running Bitwarden agent is not consulted as well: this session points at another
        // socket, and a key the user did not point it at must not sign.
        let sockets = candidates(None, &env);
        assert_eq!(sockets.len(), 1, "{sockets:?}");
        assert_eq!(sockets[0].path, env.socket.clone().unwrap());
        assert_eq!(sockets[0].kind, AgentKind::System);
        assert!(!sockets.iter().any(|s| s.path == bitwarden));
    }

    #[test]
    fn a_variable_left_over_from_another_session_is_still_honoured() {
        let dir = tempfile::tempdir().unwrap();
        let bitwarden = touch(dir.path(), "home/.bitwarden-ssh-agent.sock");
        let gone = dir.path().join("home/gone.sock");
        let env = Env {
            socket: Some(gone.clone()),
            ..env_at(dir.path())
        };

        // A path that stopped working is the one used, and the error names it: the agent that is
        // running is not taken behind the user's back.
        let sockets = candidates(None, &env);
        assert_eq!(sockets.len(), 1, "{sockets:?}");
        assert_eq!(sockets[0].path, gone);
        assert!(!sockets.iter().any(|s| s.path == bitwarden));

        // Without one either, an empty list says so plainly.
        assert!(candidates(None, &Env::default()).is_empty());
    }

    #[test]
    fn the_launchd_placeholder_of_a_gui_app_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let bitwarden = touch(dir.path(), "home/.bitwarden-ssh-agent.sock");
        // macOS points SSH_AUTH_SOCK of a window opened from Finder at a listener of launchd's,
        // which is not an agent: connecting to it only asks launchd to start an empty one. Both
        // shapes launchd hands out are skipped, and the agent beside HOME is what remains.
        for placeholder in [
            "/var/run/com.apple.launchd.jy4KjswD0t/Listeners",
            "/private/tmp/com.apple.launchd.abc123/Listeners",
        ] {
            assert!(
                is_launchd_placeholder(Path::new(placeholder)),
                "{placeholder}"
            );
            let env = Env {
                socket: Some(PathBuf::from(placeholder)),
                ..env_at(dir.path())
            };
            let sockets = candidates(None, &env);
            assert_eq!(sockets.len(), 1, "{placeholder}: {sockets:?}");
            assert_eq!(sockets[0].path, bitwarden, "{placeholder}");
        }
        assert!(!is_launchd_placeholder(Path::new(
            "/Users/x/.bitwarden-ssh-agent.sock"
        )));
    }

    #[test]
    fn the_sockets_of_running_agents_are_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (
                "home/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock",
                AgentKind::OnePassword,
            ),
            ("home/.gnupg/S.gpg-agent.ssh", AgentKind::GnuPgp),
            ("run/gnupg/S.gpg-agent.ssh", AgentKind::GnuPgp),
            ("run/keyring/ssh", AgentKind::GnomeKeyring),
            (
                "home/Library/Containers/com.bitwarden.desktop/Data/.bitwarden-ssh-agent.sock",
                AgentKind::Bitwarden,
            ),
            (
                "home/.var/app/com.bitwarden.desktop/data/.bitwarden-ssh-agent.sock",
                AgentKind::Bitwarden,
            ),
        ];
        for (rel, kind) in cases {
            let path = touch(dir.path(), rel);
            let socket = candidates(None, &env_at(dir.path()))
                .into_iter()
                .find(|s| s.path == path)
                .unwrap_or_else(|| panic!("{rel} was not guessed"));
            assert_eq!(socket.kind, kind, "{rel}");
        }

        // A socket of an agent that is not running, and one outside the home and runtime
        // directory, are not guessed.
        let guessed = candidates(None, &env_at(dir.path()));
        assert!(
            !guessed
                .iter()
                .any(|s| s.path.to_string_lossy().contains("snap/bitwarden"))
        );
        assert!(candidates(None, &Env::default()).is_empty());
    }

    #[test]
    fn gnupg_home_is_used_when_it_is_set() {
        let dir = tempfile::tempdir().unwrap();
        let socket = touch(dir.path(), "elsewhere/S.gpg-agent.ssh");
        let env = Env {
            gnupg: Some(dir.path().join("elsewhere")),
            ..Env::default()
        };
        let guessed = candidates(None, &env);
        assert_eq!(guessed.len(), 1);
        assert_eq!(guessed[0].path, socket);
        assert_eq!(guessed[0].kind, AgentKind::GnuPgp);
    }

    #[test]
    fn without_a_socket_the_error_names_what_looked_and_how_to_fix_it() {
        let dir = tempfile::tempdir().unwrap();
        let socket = AgentSocket::new(dir.path().join("home/.bitwarden-ssh-agent.sock"));

        let err = no_agent(&[socket], None);
        assert_eq!(err.kind, crate::error::SshErrorKind::Other);
        assert!(err.message.contains("no ssh-agent answered at"), "{err}");
        assert!(err.message.contains(".bitwarden-ssh-agent.sock"), "{err}");

        let err = no_agent(&[], None);
        assert!(err.message.contains("SSH_AUTH_SOCK"), "{err}");
        assert!(err.message.contains("Bitwarden"), "{err}");
        assert!(err.message.contains("Settings → SSH Agent"), "{err}");
    }
}

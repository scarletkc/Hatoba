//! ssh-agent access (SSH-09).
//!
//! A hop without sockets of its own ([`JumpHop::agent_sockets`], [`ConnectConfig::agent_sockets`])
//! uses the platform's agent:
//! Unix (Linux/macOS): the Unix socket named by `SSH_AUTH_SOCK`.
//! Windows: the OpenSSH agent named pipe `\\.\pipe\openssh-ssh-agent`
//! (or the pipe named by `SSH_AUTH_SOCK` if it points at one).
//!
//! Sockets given to a hop are tried in order. One counts only when it answers the agent protocol,
//! and the first that holds identities is used; when none holds any, the first that answered is,
//! so the caller reports an empty agent as it always did.
//!
//! [`JumpHop::agent_sockets`]: crate::JumpHop::agent_sockets
//! [`ConnectConfig::agent_sockets`]: crate::ConnectConfig::agent_sockets

use std::time::Duration;

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};

use crate::error::SshError;

/// A connected agent, with the stream type erased so the platform paths agree.
pub(crate) type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

/// Name of the Win32-OpenSSH agent pipe.
#[cfg(windows)]
const WINDOWS_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// How long a given socket has to accept the connection, and then to list its identities: a
/// socket that accepts connections without being an agent must not hold the connection up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Connects to the agent a hop signs in with and lists the identities it holds.
pub(crate) async fn connect(sockets: &[String]) -> Result<(Agent, Vec<AgentIdentity>), SshError> {
    if sockets.is_empty() {
        let mut agent = connect_platform().await?;
        let identities = agent
            .request_identities()
            .await
            .map_err(|e| SshError::other(format!("ssh-agent request failed: {e}")))?;
        return Ok((agent, identities));
    }
    let mut empty: Option<Agent> = None;
    let mut failures: Vec<(&str, String)> = Vec::new();
    for socket in sockets {
        let no_answer = || format!("no answer in {} seconds", PROBE_TIMEOUT.as_secs());
        // The connect is bounded as well as the request: a busy Windows named pipe retries inside
        // the client with no deadline of its own.
        let mut agent = match tokio::time::timeout(PROBE_TIMEOUT, open(socket)).await {
            Ok(Ok(agent)) => agent,
            Ok(Err(reason)) => {
                failures.push((socket, reason));
                continue;
            }
            Err(_) => {
                failures.push((socket, no_answer()));
                continue;
            }
        };
        match tokio::time::timeout(PROBE_TIMEOUT, agent.request_identities()).await {
            Ok(Ok(identities)) if !identities.is_empty() => return Ok((agent, identities)),
            Ok(Ok(_)) => {
                empty.get_or_insert(agent);
            }
            Ok(Err(e)) => failures.push((socket, e.to_string())),
            Err(_) => failures.push((socket, no_answer())),
        }
    }
    if let Some(agent) = empty {
        return Ok((agent, Vec::new()));
    }
    Err(not_available(&failures))
}

/// Why none of the sockets answered, each with its own reason.
fn not_available(failures: &[(&str, String)]) -> SshError {
    let message = match failures {
        [(socket, reason)] => format!("ssh-agent is not available at {socket}: {reason}"),
        _ => {
            let each: Vec<String> = failures
                .iter()
                .map(|(socket, reason)| format!("{socket} ({reason})"))
                .collect();
            format!("ssh-agent is not available at {}", each.join(" or "))
        }
    };
    SshError::other(message)
}

#[cfg(unix)]
async fn connect_platform() -> Result<Agent, SshError> {
    let client = AgentClient::connect_env()
        .await
        .map_err(|e| SshError::other(format!("ssh-agent is not available: {e}")))?;
    Ok(client.dynamic())
}

#[cfg(windows)]
async fn connect_platform() -> Result<Agent, SshError> {
    let pipe = std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|p| p.starts_with(r"\\.\pipe\"))
        .unwrap_or_else(|| WINDOWS_AGENT_PIPE.to_owned());
    let client = AgentClient::connect_named_pipe(&pipe).await.map_err(|e| {
        SshError::other(format!(
            "ssh-agent is not available (is the \"OpenSSH Authentication Agent\" service running?): {e}"
        ))
    })?;
    Ok(client.dynamic())
}

#[cfg(not(any(unix, windows)))]
async fn connect_platform() -> Result<Agent, SshError> {
    Err(SshError::other(
        "ssh-agent is not supported on this platform",
    ))
}

/// Opens one given socket. The error is the reason to report.
#[cfg(unix)]
async fn open(socket: &str) -> Result<Agent, String> {
    AgentClient::connect_uds(socket)
        .await
        .map(AgentClient::dynamic)
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
async fn open(socket: &str) -> Result<Agent, String> {
    AgentClient::connect_named_pipe(socket)
        .await
        .map(AgentClient::dynamic)
        .map_err(|e| e.to_string())
}

#[cfg(not(any(unix, windows)))]
async fn open(_socket: &str) -> Result<Agent, String> {
    Err("ssh-agent is not supported on this platform".to_owned())
}

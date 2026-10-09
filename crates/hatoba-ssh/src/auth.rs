//! Authentication of one SSH hop: password, private key, agent, none,
//! keyboard-interactive and agent-then-ask (SSH-01, SSH-02, SSH-08, SSH-09).

use std::sync::Arc;

use russh::MethodKind;
use russh::MethodSet;
use russh::client::{Handle, KeyboardInteractiveAuthResponse};
use russh::keys::PrivateKeyWithHashAlg;
use russh::keys::agent::AgentIdentity;
use russh::keys::ssh_key::HashAlg;
use zeroize::Zeroizing;

use crate::agent;
use crate::error::{SshError, SshErrorKind};
use crate::handler::{ClientHandler, CloseReason, SessionShared};
use crate::interactive::{KeyboardInteractive, Prompt, PromptRequest};
use crate::keys;
use crate::net::ConnectClock;
use crate::session::AuthMethod;

/// Safety valve against servers that keep asking forever.
const MAX_KI_ROUNDS: usize = 10;

pub(crate) struct AuthContext<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub interactive: Option<&'a Arc<dyn KeyboardInteractive>>,
    pub clock: &'a Arc<ConnectClock>,
    pub shared: &'a Arc<SessionShared>,
}

/// Authenticates `handle` as `ctx.username`. On failure the error is
/// classified ([`SshErrorKind::AuthFailed`], [`SshErrorKind::Cancelled`], ...).
pub(crate) async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthContext<'_>,
    auth: &AuthMethod,
) -> Result<(), SshError> {
    // Probe with "none": it succeeds on open servers and otherwise tells us
    // which methods are offered, so we do not burn attempts on methods the
    // server has disabled.
    let offered = match handle.authenticate_none(ctx.username).await? {
        russh::client::AuthResult::Success => return Ok(()),
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => {
            fail_if_disconnected(ctx, &remaining_methods)?;
            remaining_methods
        }
    };

    match auth {
        AuthMethod::None => Err(auth_failed(
            "the server requires authentication",
            Some(&offered),
        )),
        AuthMethod::Password(password) => password_auth(handle, ctx, password, &offered).await,
        AuthMethod::PrivateKey {
            openssh,
            passphrase,
        } => {
            let key =
                keys::load_for_auth(openssh.as_str(), passphrase.as_ref().map(|p| p.as_str()))?;
            let key = Arc::new(key);
            let hash = if key.algorithm().is_rsa() {
                handle.best_supported_rsa_hash().await?.flatten()
            } else {
                None
            };
            let result = handle
                .authenticate_publickey(ctx.username, PrivateKeyWithHashAlg::new(key, hash))
                .await?;
            finish(handle, ctx, result, "the server rejected the private key").await
        }
        AuthMethod::Agent => agent_auth(handle, ctx).await,
        AuthMethod::AgentThenAsk => agent_then_ask(handle, ctx, &offered).await,
    }
}

/// [`AuthMethod::AgentThenAsk`]. Like `ssh`, keyboard-interactive is preferred over asking for
/// the password, since the server's own prompts may ask for more than a password.
async fn agent_then_ask(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthContext<'_>,
    offered: &MethodSet,
) -> Result<(), SshError> {
    if offered.contains(&MethodKind::PublicKey) {
        match agent_auth(handle, ctx).await {
            Ok(()) => return Ok(()),
            // No agent, an empty one, or no identity accepted: go on with the next method.
            Err(e) if matches!(e.kind, SshErrorKind::AuthFailed | SshErrorKind::Other) => {
                tracing::debug!("ssh-agent did not authenticate: {}", e.message);
            }
            Err(e) => return Err(e),
        }
    }
    if offered.contains(&MethodKind::KeyboardInteractive) {
        return keyboard_interactive(handle, ctx, None).await;
    }
    if !offered.contains(&MethodKind::Password) {
        return Err(auth_failed(
            "no ssh-agent identity was accepted",
            Some(offered),
        ));
    }
    let Some(callback) = ctx.interactive else {
        return Err(SshError::new(
            SshErrorKind::AuthFailed,
            "the server asks for a password but no handler is available",
        ));
    };
    let request = PromptRequest {
        host: ctx.host.to_owned(),
        port: ctx.port,
        username: ctx.username.to_owned(),
        password: true,
        name: String::new(),
        instructions: String::new(),
        prompts: vec![Prompt {
            text: "Password: ".to_owned(),
            echo: false,
        }],
    };
    let answers = {
        let _pause = ctx.clock.pause();
        callback.respond(request).await
    };
    let Some(password) = answers.and_then(|a| a.into_iter().next()) else {
        return Err(SshError::cancelled());
    };
    let result = handle
        .authenticate_password(ctx.username, password.as_str())
        .await?;
    finish(handle, ctx, result, "the server rejected the password").await
}

async fn password_auth(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthContext<'_>,
    password: &Zeroizing<String>,
    offered: &MethodSet,
) -> Result<(), SshError> {
    let has_password = offered.contains(&MethodKind::Password);
    let has_ki = offered.contains(&MethodKind::KeyboardInteractive);

    if has_password || !has_ki {
        let result = handle
            .authenticate_password(ctx.username, password.as_str())
            .await?;
        return finish(handle, ctx, result, "the server rejected the password").await;
    }
    // PasswordAuthentication is off but keyboard-interactive (PAM) is on:
    // the first password-looking prompt is answered with the saved password.
    keyboard_interactive(handle, ctx, Some(password)).await
}

/// Interprets an authentication result: success, continue with a second
/// factor (partial success), or a classified failure.
async fn finish(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthContext<'_>,
    result: russh::client::AuthResult,
    rejected: &str,
) -> Result<(), SshError> {
    match result {
        russh::client::AuthResult::Success => Ok(()),
        russh::client::AuthResult::Failure {
            remaining_methods,
            partial_success,
        } => {
            fail_if_disconnected(ctx, &remaining_methods)?;
            if partial_success && remaining_methods.contains(&MethodKind::KeyboardInteractive) {
                keyboard_interactive(handle, ctx, None).await
            } else {
                Err(auth_failed(rejected, Some(&remaining_methods)))
            }
        }
    }
}

/// russh reports a dropped connection as "failure with no methods left".
fn fail_if_disconnected(ctx: &AuthContext<'_>, remaining: &MethodSet) -> Result<(), SshError> {
    if !remaining.is_empty() || !ctx.shared.is_closed() {
        return Ok(());
    }
    Err(match ctx.shared.reason() {
        Some(CloseReason::Error(e)) => e.with_context("connection lost during authentication"),
        Some(CloseReason::Remote(msg)) => {
            SshError::disconnected(format!("{msg} (during authentication)"))
        }
        _ => SshError::disconnected("the server closed the connection during authentication"),
    })
}

fn auth_failed(what: &str, offered: Option<&MethodSet>) -> SshError {
    let methods = offered
        .filter(|m| !m.is_empty())
        .map(|m| m.iter().map(<&str>::from).collect::<Vec<_>>().join(", "))
        .map(|m| format!(" (server accepts: {m})"))
        .unwrap_or_default();
    SshError::new(SshErrorKind::AuthFailed, format!("{what}{methods}"))
}

/// Runs keyboard-interactive rounds. If `password` is given, the first lone
/// password-looking prompt is answered with it (once); everything else goes to
/// the [`KeyboardInteractive`] callback.
async fn keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthContext<'_>,
    password: Option<&Zeroizing<String>>,
) -> Result<(), SshError> {
    let mut password_unused = password;
    let mut response = handle
        .authenticate_keyboard_interactive_start(ctx.username, None)
        .await?;

    for _ in 0..MAX_KI_ROUNDS {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure {
                remaining_methods, ..
            } => {
                fail_if_disconnected(ctx, &remaining_methods)?;
                return Err(auth_failed(
                    if password.is_some() {
                        "the server rejected the password (keyboard-interactive)"
                    } else {
                        "keyboard-interactive authentication failed"
                    },
                    Some(&remaining_methods),
                ));
            }
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let answers: Vec<Zeroizing<String>> = if prompts.is_empty() {
                    Vec::new()
                } else if is_lone_password_prompt(&prompts)
                    && let Some(pw) = password_unused.take()
                {
                    vec![Zeroizing::new(pw.to_string())]
                } else {
                    let Some(callback) = ctx.interactive else {
                        return Err(SshError::new(
                            SshErrorKind::AuthFailed,
                            "the server requires interactive input (for example a verification code) but no handler is available",
                        ));
                    };
                    let request = PromptRequest {
                        host: ctx.host.to_owned(),
                        port: ctx.port,
                        username: ctx.username.to_owned(),
                        password: false,
                        name,
                        instructions,
                        prompts: prompts
                            .into_iter()
                            .map(|p| Prompt {
                                text: p.prompt,
                                echo: p.echo,
                            })
                            .collect(),
                    };
                    let expected = request.prompts.len();
                    let _pause = ctx.clock.pause();
                    let Some(answers) = callback.respond(request).await else {
                        return Err(SshError::cancelled());
                    };
                    if answers.len() != expected {
                        return Err(SshError::other(
                            "keyboard-interactive handler returned the wrong number of answers",
                        ));
                    }
                    answers
                };
                response = handle
                    .authenticate_keyboard_interactive_respond(
                        answers.iter().map(|a| a.as_str().to_owned()).collect(),
                    )
                    .await?;
            }
        }
    }
    Err(SshError::new(
        SshErrorKind::AuthFailed,
        "too many keyboard-interactive rounds",
    ))
}

fn is_lone_password_prompt(prompts: &[russh::client::Prompt]) -> bool {
    prompts.len() == 1 && prompts[0].prompt.to_ascii_lowercase().contains("password")
}

async fn agent_auth(
    handle: &mut Handle<ClientHandler>,
    ctx: &AuthContext<'_>,
) -> Result<(), SshError> {
    let mut agent = agent::connect().await?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| SshError::other(format!("ssh-agent request failed: {e}")))?;
    if identities.is_empty() {
        return Err(SshError::new(
            SshErrorKind::AuthFailed,
            "ssh-agent holds no identities",
        ));
    }
    let total = identities.len();
    for identity in identities {
        let outcome = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                let hash = rsa_hash(handle, key.algorithm().is_rsa()).await;
                handle
                    .authenticate_publickey_with(ctx.username, key, hash, &mut agent)
                    .await
            }
            AgentIdentity::Certificate { certificate, .. } => {
                let hash = rsa_hash(handle, certificate.algorithm().is_rsa()).await;
                handle
                    .authenticate_certificate_with(ctx.username, certificate, hash, &mut agent)
                    .await
            }
        };
        match outcome {
            Ok(russh::client::AuthResult::Success) => return Ok(()),
            Ok(result) => {
                // A partial success may still require a second factor.
                if let russh::client::AuthResult::Failure {
                    partial_success: true,
                    ref remaining_methods,
                } = result
                    && remaining_methods.contains(&MethodKind::KeyboardInteractive)
                {
                    return keyboard_interactive(handle, ctx, None).await;
                }
                if let russh::client::AuthResult::Failure {
                    remaining_methods, ..
                } = &result
                {
                    fail_if_disconnected(ctx, remaining_methods)?;
                }
            }
            Err(e) => tracing::debug!("ssh-agent identity failed: {e}"),
        }
    }
    Err(SshError::new(
        SshErrorKind::AuthFailed,
        format!("none of the {total} ssh-agent identities was accepted by the server"),
    ))
}

/// Best RSA signature hash the server accepts (`None` = legacy `ssh-rsa`).
async fn rsa_hash(handle: &Handle<ClientHandler>, is_rsa: bool) -> Option<HashAlg> {
    if is_rsa {
        handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten()
    } else {
        None
    }
}

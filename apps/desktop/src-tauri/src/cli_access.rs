//! Other programs use the saved SSH hosts through the `hatoba` command (HOST-15): `hatoba hosts`
//! lists them, `hatoba exec` runs a command on one, `hatoba cp` copies a file to or from one.
//! The command asks this app over a local endpoint (`hatoba_cli::endpoint`), which only the user
//! can open; the app signs in with the vault's credentials and sends back only what the command
//! printed or the file it copied. It listens while Settings → General has both the command and
//! this setting on. The vault has to be unlocked, and the lock stops what runs.
//!
//! Questions a connection has for the user (an untrusted host key, a password that is not saved, a
//! code) appear in the window as they do for a terminal, which comes to the front, and the command
//! says what it waits for; `--no-prompt` fails instead.
//!
//! `--sudo` runs the command, or reads or writes the remote file, as root (`hatoba_ssh::sudo`).
//! When sudo asks for a password, the one given in the window for the host within the last 15
//! minutes goes first, then the host's saved password, then the window asks, as for a connection.
//!
//! `hatoba memory` lists, saves and deletes the AI assistant's memories of a host ([`memories`]).

mod exec;
mod hosts;
mod memories;
mod pool;
mod server;
#[cfg(test)]
mod tests;
mod transfer;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use hatoba_cli::SWITCH;
use hatoba_cli::protocol::{End, Frame};
use hatoba_core::model::{HostAuth, Item};
use hatoba_ssh::sudo::{self, Access, AccessError, Checked, RunAs};
use hatoba_ssh::{SshSession, target_label};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::io::AsyncRead;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::dto::{
    CliActivity, CliActivityEvent, CliActivityKind, CliActivityState, HostKeyPromptKind,
};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::ssh::{self, Asking, OnAsk, OnBanner, Overrides, Unattended};
use crate::state::{MutexExt, now_ms, state};

pub(crate) use hosts::{find_host, host_list};
pub use server::sync;

/// How long the command may take to send its request once connected.
const REQUEST_WAIT: Duration = Duration::from_secs(10);
/// How often idle connections are looked for.
const IDLE_CHECK: Duration = Duration::from_secs(60);
/// How many uses Settings lists.
const ACTIVITY: usize = 20;
/// How long a sudo password given in the window is kept after its host's last `--sudo`.
const SUDO_KEPT: Duration = Duration::from_secs(15 * 60);
/// How many sudo passwords the window asks for before a request fails.
const SUDO_TRIES: usize = 3;

const LOCKED: &str = "Hatoba is locked. Unlock it, then run the command again.";

#[derive(Default)]
pub struct CliAccess {
    /// Stops listening; `None` while the setting is off.
    listener: Mutex<Option<CancellationToken>>,
    /// Ends with the lock and when the setting goes off: what runs then stops.
    epoch: Mutex<CancellationToken>,
    pool: pool::Pool,
    /// Newest first.
    activity: Mutex<VecDeque<CliActivity>>,
    next_id: AtomicU64,
    /// The sudo passwords given in the window, by host and the way it connects ([`sudo_key`]).
    sudo: Mutex<HashMap<String, KeptPassword>>,
    /// Lets one request at a time find the sudo password for a [`sudo_key`], so the others wait
    /// and take the one it found instead of asking again.
    sudo_turns: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The `hatoba exec` commands that wait for the connection with their stdin, by the token
    /// [`Frame::SendStdin`] gave it.
    stdin: Mutex<HashMap<String, oneshot::Sender<Input>>>,
}

/// The connection that brings a command's stdin, after its request.
type Input = Box<dyn AsyncRead + Send + Unpin>;

struct KeptPassword {
    password: Zeroizing<String>,
    /// The clock's time, not an `Instant`, so that time asleep counts, as it does for sudo.
    used: SystemTime,
}

impl CliAccess {
    fn epoch(&self) -> CancellationToken {
        self.epoch.lock_unpoisoned().clone()
    }

    /// Stops every command and copy, closes the connections, and forgets the sudo passwords.
    fn end_epoch(&self) {
        std::mem::take(&mut *self.epoch.lock_unpoisoned()).cancel();
        self.pool.close_all();
        self.sudo.lock_unpoisoned().clear();
        self.sudo_turns.lock_unpoisoned().clear();
        self.stdin.lock_unpoisoned().clear();
    }

    fn sudo_turn(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.sudo_turns
            .lock_unpoisoned()
            .entry(key.to_owned())
            .or_default()
            .clone()
    }

    /// The sudo password given in the window for `key`, unless [`SUDO_KEPT`] has passed since it
    /// was last used, or the clock went back.
    fn kept_sudo(&self, key: &str) -> Option<Zeroizing<String>> {
        let mut kept = self.sudo.lock_unpoisoned();
        kept.retain(|_, k| k.used.elapsed().is_ok_and(|e| e < SUDO_KEPT));
        kept.get(key).map(|k| k.password.clone())
    }

    fn keep_sudo(&self, key: String, password: Zeroizing<String>) {
        self.sudo.lock_unpoisoned().insert(
            key,
            KeptPassword {
                password,
                used: SystemTime::now(),
            },
        );
    }

    fn forget_sudo(&self, key: &str) {
        self.sudo.lock_unpoisoned().remove(key);
    }

    /// SEC-03: nothing runs behind the lock screen, no connection stays, and the list of what ran
    /// is forgotten.
    pub fn lock(&self, app: &AppHandle) {
        self.end_epoch();
        self.activity.lock_unpoisoned().clear();
        self.emit(app);
    }

    pub fn activity(&self) -> Vec<CliActivity> {
        self.activity.lock_unpoisoned().iter().cloned().collect()
    }

    fn emit(&self, app: &AppHandle) {
        let _ = CliActivityEvent {
            entries: self.activity(),
        }
        .emit(app);
    }

    fn begin(
        &self,
        app: &AppHandle,
        kind: CliActivityKind,
        host: &Target,
        detail: String,
    ) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        {
            let mut activity = self.activity.lock_unpoisoned();
            activity.push_front(CliActivity {
                id: id.clone(),
                kind,
                host_id: host.id.clone(),
                host_name: host.name.clone(),
                detail,
                started_at: now_ms(),
                state: CliActivityState::Running,
                status: None,
                error: None,
            });
            activity.truncate(ACTIVITY);
        }
        self.emit(app);
        id
    }

    fn finish(&self, app: &AppHandle, id: &str, end: &End) {
        {
            let mut activity = self.activity.lock_unpoisoned();
            // Gone after a lock.
            let Some(entry) = activity.iter_mut().find(|e| e.id == id) else {
                return;
            };
            entry.state = if end.error.is_some() {
                CliActivityState::Failed
            } else {
                CliActivityState::Done
            };
            entry.status = end.status;
            entry.error = end.error.clone();
        }
        self.emit(app);
    }
}

/// The host a request is for.
struct Target {
    id: String,
    name: String,
    /// How it connects now (`host_chain::config_version`).
    config: String,
}

/// What a request runs with.
struct Ctx<'a> {
    app: &'a AppHandle,
    tx: &'a mpsc::Sender<Frame>,
    epoch: CancellationToken,
    /// The command closed its end.
    gone: &'a CancellationToken,
    /// Questions for the user go to the window; otherwise a connection that has one fails.
    prompt: bool,
}

impl Ctx<'_> {
    fn target(&self, wanted: &str) -> Result<Target, String> {
        state(self.app)
            .with_unlocked(|v| {
                Ok(find_host(v, wanted).map(|(id, host)| Target {
                    config: crate::host_chain::config_version(v, &host),
                    name: host.name,
                    id,
                }))
            })
            .unwrap_or_else(|_| Err(LOCKED.to_owned()))
    }

    async fn stopped(&self) {
        tokio::select! {
            () = self.epoch.cancelled() => {}
            () = self.gone.cancelled() => {}
        }
    }

    fn is_stopped(&self) -> bool {
        self.epoch.is_cancelled() || self.gone.is_cancelled()
    }

    /// Why `what` stopped before its end.
    fn stop_message(&self, what: &str) -> String {
        if !self.epoch.is_cancelled() {
            format!("the hatoba command ended, which stopped {what}.")
        } else if state(self.app).vault().is_unlocked() {
            // Turning off the command itself, the switch above, ends it too.
            format!(
                "\"{SWITCH}\" or the hatoba command was turned off in Hatoba, which stopped {what}."
            )
        } else {
            format!("Hatoba locked, which stopped {what}.")
        }
    }

    fn begin(&self, kind: CliActivityKind, target: &Target, detail: String) -> String {
        state(self.app).cli.begin(self.app, kind, target, detail)
    }

    fn finish(&self, id: &str, end: &End) {
        state(self.app).cli.finish(self.app, id, end);
    }

    /// The host's connection: a terminal's that connects the way the host does now, as the
    /// dashboard takes it (HOST-14), or the pool's.
    async fn lease(&self, target: &Target) -> Result<pool::Lease, String> {
        let state = state(self.app);
        if let Some(session) = state.connections.session_for(&target.id, &target.config) {
            // The pool's own connection to the host goes once the commands on it are done.
            state.cli.pool.release(&target.id);
            return Ok(pool::Lease::borrowed(session));
        }
        let leasing = state
            .cli
            .pool
            .lease(&target.id, &target.config, &self.epoch, || {
                self.connect(target)
            });
        tokio::select! {
            leased = leasing => leased.map_err(|e| {
                if self.epoch.is_cancelled() {
                    self.stop_message("the connection")
                } else {
                    e.detail
                }
            }),
            () = self.stopped() => Err(self.stop_message("the connection")),
        }
    }

    /// Connects to the host. The error's detail is the whole message for the command.
    async fn connect(&self, target: &Target) -> AppResult<SshSession> {
        let overrides = Overrides {
            ask_password: self.prompt,
            ..Overrides::default()
        };
        let cfg = state(self.app)
            .with_unlocked(|v| {
                let host = v
                    .get(&target.id)
                    .and_then(Item::as_host)
                    .cloned()
                    .ok_or_else(|| AppError::not_found("host"))?;
                ssh::build_config(v, &host, Some(&target.id), Some(&overrides))
            })
            .map_err(|e| AppError::internal(config_error(&e, &target.name)))?;
        if !self.prompt {
            let (result, outcome) =
                ssh::connect_unattended(self.app, cfg, Some(self.banner_printer())).await;
            return result
                .map_err(|e| AppError::internal(unattended_error(&e, &outcome, &target.name)));
        }
        // Weak, so a connection that keeps its prompt handlers does not keep this request open.
        let tx = self.tx.downgrade();
        let app = self.app.clone();
        let on_ask: OnAsk = Arc::new(move |asking| {
            crate::platform::window::reopen_main(&app);
            let text = match asking {
                Asking::HostKey { target } => {
                    format!("waiting for you to check the host key of {target} in Hatoba")
                }
                Asking::SignIn { target } => {
                    format!("waiting for you to sign in to {target} in Hatoba")
                }
            };
            if let Some(tx) = tx.upgrade() {
                let _ = tx.try_send(Frame::Notice(text));
            }
        });
        ssh::connect_asking(
            self.app,
            cfg,
            None,
            Some(on_ask),
            Some(self.banner_printer()),
        )
        .await
        .map_err(|e| AppError::internal(format!("can't connect to {}: {}", target.name, e.detail)))
    }

    /// Prints the server's banners on the command's stderr, as `ssh` does.
    fn banner_printer(&self) -> OnBanner {
        // Weak, as the prompt handlers are.
        let tx = self.tx.downgrade();
        Arc::new(move |text| {
            if let Some(tx) = tx.upgrade() {
                let _ = tx.try_send(Frame::Stderr(format!("{text}\n").into_bytes()));
            }
        })
    }

    /// How a `--sudo` request runs as root on the host: as the login user when that is root, or
    /// through sudo, with the password sudo asks for, if it asks for one. `what` is what a stop
    /// stops.
    async fn run_as(
        &self,
        session: &SshSession,
        target: &Target,
        what: &str,
    ) -> Result<RunAs, String> {
        let name = &target.name;
        if session.server_os() == Some(hatoba_ssh::ServerOs::Windows) {
            return Err(format!("{name} runs Windows, which has no sudo."));
        }
        let access = tokio::select! {
            access = sudo::access(session) => access,
            () = self.stopped() => return Err(self.stop_message(what)),
        };
        match access {
            Ok(Access::Root) => return Ok(RunAs::Login),
            Ok(Access::NoPassword) => return Ok(RunAs::Sudo(None)),
            Ok(Access::Password) => {}
            Err(AccessError::NoSudo) => {
                return Err(format!("{name} has no sudo, which --sudo needs."));
            }
            Err(AccessError::Failed(message)) if message.is_empty() => {
                return Err(format!("can't run sudo on {name}."));
            }
            Err(AccessError::Failed(message)) => {
                return Err(format!("can't run sudo on {name}: {message}"));
            }
            Err(AccessError::Ssh(e)) => {
                return Err(format!("can't run sudo on {name}: {}", e.message));
            }
        }
        let cli = &state(self.app).cli;
        let key = sudo_key(target);
        let _turn = tokio::select! {
            turn = cli.sudo_turn(&key).lock_owned() => turn,
            () = self.stopped() => return Err(self.stop_message(what)),
        };
        let kept = cli.kept_sudo(&key);
        if let Some(password) = kept.clone() {
            if self.sudo_takes(session, &password, target, what).await? {
                cli.keep_sudo(key, password.clone());
                return Ok(RunAs::Sudo(Some(password)));
            }
            cli.forget_sudo(&key);
        }
        let saved = state(self.app)
            .with_unlocked(|v| {
                Ok(
                    match v.get(&target.id).and_then(Item::as_host).map(|h| &h.auth) {
                        Some(HostAuth::Password { password }) if !password.is_empty() => {
                            Some(password.clone())
                        }
                        _ => None,
                    },
                )
            })
            .map_err(|_| LOCKED.to_owned())?;
        if let Some(password) = saved.filter(|p| kept.as_ref() != Some(p))
            && self.sudo_takes(session, &password, target, what).await?
        {
            return Ok(RunAs::Sudo(Some(password)));
        }
        if !self.prompt {
            return Err(format!(
                "sudo on {name} asks for a password that isn't saved. Run the command without \
                 --no-prompt to enter it in Hatoba."
            ));
        }
        let label = target_label(session.username(), session.host(), session.port());
        for attempt in 0..SUDO_TRIES {
            crate::platform::window::reopen_main(self.app);
            let _ = self.tx.try_send(Frame::Notice(format!(
                "waiting for you to enter the sudo password for {label} in Hatoba"
            )));
            let asked = tokio::select! {
                asked = ssh::ask_sudo_password(self.app, name, &label, attempt > 0) => asked,
                () = self.stopped() => return Err(self.stop_message(what)),
            };
            let Some(password) = asked else {
                return Err(format!(
                    "the sudo password for {name} wasn't entered in Hatoba."
                ));
            };
            if self.sudo_takes(session, &password, target, what).await? {
                cli.keep_sudo(key, password.clone());
                return Ok(RunAs::Sudo(Some(password)));
            }
        }
        Err(format!(
            "sudo on {name} didn't take the password {SUDO_TRIES} times."
        ))
    }

    /// Whether sudo takes `password`; an error when it refuses for another reason.
    async fn sudo_takes(
        &self,
        session: &SshSession,
        password: &str,
        target: &Target,
        what: &str,
    ) -> Result<bool, String> {
        let checked = tokio::select! {
            checked = sudo::check(session, password) => checked,
            () = self.stopped() => return Err(self.stop_message(what)),
        };
        let name = &target.name;
        match checked {
            Ok(Checked::Accepted) => Ok(true),
            Ok(Checked::Wrong) => Ok(false),
            Ok(Checked::Refused(message)) if message.is_empty() => {
                Err(format!("sudo on {name} refused."))
            }
            Ok(Checked::Refused(message)) => Err(format!("sudo on {name}: {message}")),
            Err(e) => Err(format!("can't run sudo on {name}: {}", e.message)),
        }
    }
}

/// What a sudo password is kept by: the host and the way it connects, so a host that now connects
/// as another user is asked again.
fn sudo_key(target: &Target) -> String {
    format!("{}\n{}", target.id, target.config)
}

/// What a request does in Settings' list: `text`, after `sudo` for `--sudo`.
fn detail(sudo: bool, text: String) -> String {
    if sudo { format!("sudo {text}") } else { text }
}

/// Why a connection could not be set up from the vault.
fn config_error(e: &AppError, name: &str) -> String {
    match (&e.code, e.field.as_deref()) {
        (ErrorCode::Locked, _) => LOCKED.to_owned(),
        (_, Some("password")) => format!(
            "{name} has no saved password. Run the command without --no-prompt to enter it in \
             Hatoba."
        ),
        _ => format!("can't connect to {name}: {}", e.detail),
    }
}

/// Why a connection that may ask nothing failed.
fn unattended_error(e: &AppError, outcome: &Unattended, name: &str) -> String {
    match outcome.host_key() {
        Some(HostKeyPromptKind::New) => format!(
            "the host key of {name}, or of a jump host on the way, isn't trusted yet. Run the \
             command without --no-prompt to check it in Hatoba."
        ),
        Some(HostKeyPromptKind::Changed) => format!(
            "the host key of {name}, or of a jump host on the way, has changed. Run the command \
             without --no-prompt to check it in Hatoba."
        ),
        None if outcome.prompted() && !outcome.password_prompted() => format!(
            "{name} asks for a code or a password that isn't saved. Run the command without \
             --no-prompt to answer it in Hatoba."
        ),
        None => format!("can't connect to {name}: {}", e.detail),
    }
}

//! `hatoba exec` and stdin forwarding (HOST-15).

use hatoba_cli::protocol::{End, Frame};
use hatoba_ssh::remote_command::{self, Output, Stopped};
use hatoba_ssh::sudo::RunAs;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::dto::CliActivityKind;
use crate::state::{MutexExt, state};

use super::{Ctx, Input, REQUEST_WAIT, Target, detail};

/// Why a command stopped when its stdin did not come within [`REQUEST_WAIT`].
const NO_STDIN: &str =
    "the hatoba command didn't pass its stdin within 10 seconds, which stopped the command.";

/// Passes what the connection from `take` brings to `input` as it comes, until its end. `false`
/// when none came in time, so the command got no stdin at all.
pub(super) async fn pass_stdin(
    take: oneshot::Receiver<Input>,
    input: mpsc::Sender<Vec<u8>>,
) -> bool {
    let Ok(Ok(mut stdin)) = tokio::time::timeout(REQUEST_WAIT, take).await else {
        return false;
    };
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        match stdin.read(&mut buf).await {
            Ok(0) | Err(_) => return true,
            Ok(n) => {
                if input.send(buf[..n].to_vec()).await.is_err() {
                    return true;
                }
            }
        }
    }
}

impl Ctx<'_> {
    /// Asks the command for its stdin under `token`, and passes it to `input` as it comes, until
    /// its end. `false` when no connection brought it in time.
    async fn take_stdin(&self, token: &str, input: mpsc::Sender<Vec<u8>>) -> bool {
        let (give, take) = oneshot::channel();
        state(self.app)
            .cli
            .stdin
            .lock_unpoisoned()
            .insert(token.to_owned(), give);
        // Without a reader the hatoba command is gone, which stops the command it ran.
        if self
            .tx
            .send(Frame::SendStdin(token.to_owned()))
            .await
            .is_err()
        {
            return true;
        }
        pass_stdin(take, input).await
    }
}

pub(super) async fn exec(
    ctx: &Ctx<'_>,
    host: &str,
    command: String,
    cwd: Option<String>,
    sudo: bool,
    stdin: bool,
) -> End {
    let target = match ctx.target(host) {
        Ok(target) => target,
        Err(message) => return End::error(message),
    };
    let id = ctx.begin(
        CliActivityKind::Exec,
        &target,
        detail(sudo, command.clone()),
    );
    let end = run_exec(ctx, &target, &command, cwd.as_deref(), sudo, stdin).await;
    ctx.finish(&id, &end);
    end
}

async fn run_exec(
    ctx: &Ctx<'_>,
    target: &Target,
    command: &str,
    cwd: Option<&str>,
    sudo: bool,
    stdin: bool,
) -> End {
    let lease = match ctx.lease(target).await {
        Ok(lease) => lease,
        Err(message) => return End::error(message),
    };
    if cwd.is_some() && lease.session.server_os() == Some(hatoba_ssh::ServerOs::Windows) {
        return End::error(format!(
            "{} runs commands with cmd.exe or PowerShell, so --cwd cannot start one in another \
             directory. Leave out --cwd and change the directory in the command.",
            target.name
        ));
    }
    let run_as = if sudo {
        match ctx.run_as(&lease.session, target, "the command").await {
            Ok(run_as) => run_as,
            Err(message) => return End::error(message),
        }
    } else {
        RunAs::Login
    };
    let (input, stdin) = match stdin {
        true => {
            let (tx, rx) = mpsc::channel(4);
            (Some(tx), Some(rx))
        }
        false => (None, None),
    };
    let token = uuid::Uuid::new_v4().simple().to_string();
    // The command's stdin did not come, and an empty one in its place would run it on input it
    // was not given.
    let no_stdin = CancellationToken::new();
    let (output, mut pieces) = mpsc::channel(16);
    let run = remote_command::run_streaming_as(
        &lease.session,
        command,
        cwd,
        &run_as,
        stdin,
        async {
            tokio::select! {
                () = ctx.stopped() => {}
                () = no_stdin.cancelled() => {}
            }
        },
        output,
    );
    let pass_on = async {
        while let Some(piece) = pieces.recv().await {
            let frame = match piece {
                Output::Stdout(bytes) => Frame::Stdout(bytes.to_vec()),
                Output::Stderr(bytes) => Frame::Stderr(bytes.to_vec()),
            };
            // Without a reader, the command is stopping already.
            let _ = ctx.tx.send(frame).await;
        }
    };
    let pass_in = async {
        if let Some(input) = input
            && !ctx.take_stdin(&token, input).await
        {
            no_stdin.cancel();
        }
        std::future::pending::<()>().await;
    };
    let (ran, ()) = tokio::select! {
        ran = async { tokio::join!(run, pass_on) } => ran,
        () = pass_in => unreachable!("passing the stdin never ends"),
    };
    state(ctx.app).cli.stdin.lock_unpoisoned().remove(&token);
    match ran {
        Ok(run) => match (run.stopped, run.output.exit_status) {
            (None, Some(status)) => End::status(i32::try_from(status).unwrap_or(i32::MAX)),
            (None, None) => {
                End::error("the command ended without an exit status, as when a signal ends it")
            }
            (Some(stopped), _) => {
                let mut message = if ctx.is_stopped() {
                    ctx.stop_message("the command")
                } else {
                    NO_STDIN.to_owned()
                };
                if stopped == Stopped::MayRun {
                    message.push_str(" It may still be running on the host.");
                }
                End::error(message)
            }
        },
        Err(e) if lease.session.is_closed() => End::error(format!(
            "the connection to {} was lost: {}",
            target.name, e.message
        )),
        Err(e) => End::error(e.message),
    }
}

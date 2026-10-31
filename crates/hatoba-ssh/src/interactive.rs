//! The user's side of signing in: keyboard-interactive prompts (SSH-08) and the messages the
//! server sends while signing in.

use async_trait::async_trait;
use futures::future::BoxFuture;
use zeroize::Zeroizing;

/// One prompt of a keyboard-interactive round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// Text to show to the user, e.g. `Verification code: `.
    pub text: String,
    /// Whether the answer may be echoed while typing (`false` for passwords/OTP).
    pub echo: bool,
}

/// A keyboard-interactive info request sent by the server, or the login password that
/// [`AuthMethod::AgentThenAsk`](crate::AuthMethod::AgentThenAsk) asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRequest {
    /// Host (as configured for the hop) that is asking.
    pub host: String,
    /// SSH port of that host.
    pub port: u16,
    /// Login name on that host.
    pub username: String,
    /// `true` when the client asks for the login password itself rather than relaying a
    /// server prompt: `prompts` then holds one prompt that is not echoed.
    pub password: bool,
    /// Optional title chosen by the server (often empty).
    pub name: String,
    /// Optional instructions chosen by the server (often empty).
    pub instructions: String,
    /// Prompts to answer, in order. Never empty when passed to the callback.
    pub prompts: Vec<Prompt>,
}

/// A message the server sent while signing in (`SSH_MSG_USERAUTH_BANNER`): a notice such as
/// OpenSSH's `Banner`, or what to do for a server that holds the login until the user acts, such
/// as the link that Tailscale SSH's check mode asks to sign in at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    /// Host (as configured for the hop) that sent it.
    pub host: String,
    /// SSH port of that host.
    pub port: u16,
    /// Login name on that host.
    pub username: String,
    /// The message, never empty. Lines end with `\n`, and the only other control character
    /// kept is tab, so the server cannot drive a terminal with it.
    pub text: String,
}

/// What [`KeyboardInteractive::banner`] returns for a banner someone sees.
pub type BannerWait = BoxFuture<'static, ()>;

/// Answers keyboard-interactive prompts (2FA / OTP / PAM challenges) and shows the server's
/// banners.
///
/// The connect timeout is paused while the callback is running.
#[async_trait]
pub trait KeyboardInteractive: Send + Sync {
    /// Returns exactly one answer per prompt, or `None` to abort the login
    /// (the connection then fails with `SshErrorKind::Cancelled`).
    async fn respond(&self, request: PromptRequest) -> Option<Vec<Zeroizing<String>>>;

    /// Shows `banner`. Returns `None` when nobody sees it, as the default does. Otherwise the
    /// returned future runs until the server answers the request the banner came with, and is
    /// then dropped. Meanwhile the connect timeout is paused, since the server may hold the login
    /// until the user acts on the message. The future ending first cancels the login
    /// (`SshErrorKind::Cancelled`): the user stopped waiting.
    fn banner(&self, banner: Banner) -> Option<BannerWait> {
        let _ = banner;
        None
    }
}

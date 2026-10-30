//! Typing a host's saved user name and password at the server's login prompts (TELNET-03).
//!
//! Telnet has no authentication of its own: the server prints `login:` and `Password:` like any
//! other output. The saved user name is typed at a prompt that ends with `login:`, `username:`,
//! `user name:` or `user:`, and the saved password at one that ends with `password:` or
//! `passcode:`, each once and only while the prompt is the last thing on the screen. It stops
//! once the password is typed, after the user has pressed Enter twice (a user name and a password
//! of their own), or once [`LOGIN_WINDOW`] has passed, so a later `sudo` never gets the password.
//! A prompt that comes back after a wrong password is left to the user.

use std::time::Duration;

use zeroize::Zeroizing;

/// How long after connecting the prompts are answered.
pub const LOGIN_WINDOW: Duration = Duration::from_secs(120);

/// How much of the output's last line is kept to find a prompt in.
const TAIL: usize = 128;

const USER_PROMPTS: [&str; 4] = ["login:", "username:", "user name:", "user:"];
const PASSWORD_PROMPTS: [&str; 2] = ["password:", "passcode:"];

/// The credentials to type, and how far the login has got.
pub struct AutoLogin {
    username: Option<Zeroizing<String>>,
    password: Option<Zeroizing<String>>,
    /// The output's last line so far, escape sequences left out.
    line: Vec<u8>,
    /// Inside an escape sequence of the output.
    escape: Escape,
    /// Enter presses of the user's own.
    user_lines: u8,
    done: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Escape {
    None,
    /// After ESC.
    Start,
    /// In a CSI sequence, until its final byte.
    Csi,
    /// In an OSC or other string sequence, until BEL or ST.
    String,
    /// After ESC in a string sequence: a backslash ends it (ST).
    StringEsc,
}

impl AutoLogin {
    /// Types `username` and `password` where given and not empty.
    #[must_use]
    pub fn new(username: Option<String>, password: Option<String>) -> Self {
        let username = username.filter(|u| !u.is_empty()).map(Zeroizing::new);
        let password = password.filter(|p| !p.is_empty()).map(Zeroizing::new);
        let done = username.is_none() && password.is_none();
        Self {
            username,
            password,
            line: Vec::new(),
            escape: Escape::None,
            user_lines: 0,
            done,
        }
    }

    /// `true` once nothing more will be typed.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Stops typing.
    pub fn stop(&mut self) {
        self.done = true;
        self.username = None;
        self.password = None;
    }

    /// Notes the user's own input: their second Enter ends the login.
    pub fn typed(&mut self, input: &[u8]) {
        if self.done {
            return;
        }
        self.user_lines = self.user_lines.saturating_add(
            u8::try_from(input.iter().filter(|&&b| b == b'\r').count()).unwrap_or(u8::MAX),
        );
        if self.user_lines >= 2 {
            self.stop();
        } else if self.user_lines == 1 {
            // The user typed a user name of their own: the saved one is not typed after it.
            self.username = None;
            if self.password.is_none() {
                self.stop();
            }
        }
    }

    /// Reads a chunk of terminal output, and returns what to type when it ends at a prompt.
    pub fn output(&mut self, data: &[u8]) -> Option<Zeroizing<String>> {
        if self.done {
            return None;
        }
        for &byte in data {
            self.push(byte);
        }
        let text = String::from_utf8_lossy(&self.line)
            .trim_end()
            .to_lowercase();
        if PASSWORD_PROMPTS.iter().any(|p| text.ends_with(p)) {
            let password = self.password.take();
            // Whatever happens at this prompt, the login is over.
            self.stop();
            return password.map(|p| Zeroizing::new(format!("{}\r", p.as_str())));
        }
        if USER_PROMPTS.iter().any(|p| text.ends_with(p)) {
            self.line.clear();
            let username = self.username.take()?;
            if self.password.is_none() {
                self.stop();
            }
            return Some(Zeroizing::new(format!("{}\r", username.as_str())));
        }
        None
    }

    fn push(&mut self, byte: u8) {
        match self.escape {
            Escape::None => match byte {
                0x1b => self.escape = Escape::Start,
                b'\r' | b'\n' => self.line.clear(),
                // Backspace, as a server's line editing sends it.
                0x08 => {
                    self.line.pop();
                }
                0x00..=0x1f | 0x7f => {}
                _ => {
                    if self.line.len() >= TAIL {
                        self.line.remove(0);
                    }
                    self.line.push(byte);
                }
            },
            Escape::Start => {
                self.escape = match byte {
                    b'[' => Escape::Csi,
                    b']' | b'P' | b'X' | b'^' | b'_' => Escape::String,
                    _ => Escape::None,
                };
            }
            Escape::Csi => {
                if (0x40..=0x7e).contains(&byte) {
                    self.escape = Escape::None;
                }
            }
            Escape::String => match byte {
                0x07 => self.escape = Escape::None,
                0x1b => self.escape = Escape::StringEsc,
                _ => {}
            },
            Escape::StringEsc => {
                self.escape = match byte {
                    b'\\' => Escape::None,
                    0x1b => Escape::StringEsc,
                    _ => Escape::String,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(user: &str, password: &str) -> AutoLogin {
        AutoLogin::new(Some(user.into()), Some(password.into()))
    }

    fn typed(answer: Option<Zeroizing<String>>) -> Option<String> {
        answer.map(|a| a.as_str().to_owned())
    }

    #[test]
    fn types_the_user_name_then_the_password() {
        let mut l = login("admin", "s3cret");
        assert_eq!(typed(l.output(b"Welcome\r\n")), None);
        assert_eq!(typed(l.output(b"router login: ")), Some("admin\r".into()));
        assert_eq!(typed(l.output(b"admin\r\nPass")), None);
        assert_eq!(typed(l.output(b"word: ")), Some("s3cret\r".into()));
        assert!(l.is_done());
        assert_eq!(typed(l.output(b"\r\nPassword: ")), None);
    }

    #[test]
    fn prompts_match_whatever_their_case_and_colours() {
        let mut l = login("admin", "pw");
        assert_eq!(
            typed(l.output(b"\x1b[1mUser Name:\x1b[0m ")),
            Some("admin\r".into())
        );
        assert_eq!(
            typed(l.output(b"\r\n\x1b]0;title\x07PASSWORD:")),
            Some("pw\r".into())
        );
    }

    #[test]
    fn a_backslash_inside_a_title_does_not_end_it() {
        let mut l = login("admin", "pw");
        // The title holds a prompt's text, which is not on screen.
        assert_eq!(typed(l.output(b"\x1b]0;a\\Password:\x07")), None);
        assert_eq!(
            typed(l.output(b"\x1b]0;b\x1b\\Password:")),
            Some("pw\r".into())
        );
    }

    #[test]
    fn a_password_prompt_first_gets_the_password() {
        let mut l = AutoLogin::new(None, Some("line".into()));
        assert_eq!(
            typed(l.output(b"\r\nUser Access Verification\r\n\r\nPassword: ")),
            Some("line\r".into())
        );
    }

    #[test]
    fn a_login_prompt_after_a_wrong_password_is_left_to_the_user() {
        let mut l = login("admin", "wrong");
        l.output(b"login: ");
        l.output(b"Password: ");
        assert_eq!(typed(l.output(b"\r\nLogin incorrect\r\nlogin: ")), None);
    }

    #[test]
    fn a_prompt_mid_line_is_not_a_prompt() {
        let mut l = login("admin", "pw");
        assert_eq!(typed(l.output(b"Last login: Tue Oct 21")), None);
        assert_eq!(
            typed(l.output(b"\r\nchange your password: see docs\r\n")),
            None
        );
        assert!(!l.is_done());
    }

    #[test]
    fn the_users_own_user_name_still_gets_the_saved_password() {
        let mut l = login("admin", "pw");
        l.typed(b"root\r");
        assert_eq!(typed(l.output(b"login: ")), None);
        assert_eq!(typed(l.output(b"\r\nPassword: ")), Some("pw\r".into()));
    }

    #[test]
    fn two_lines_of_the_users_own_end_the_login() {
        let mut l = login("admin", "pw");
        l.typed(b"root\r");
        l.typed(b"hunter2\r");
        assert!(l.is_done());
        assert_eq!(typed(l.output(b"[sudo] password for root: ")), None);
    }

    #[test]
    fn nothing_saved_means_nothing_typed() {
        let mut l = AutoLogin::new(Some(String::new()), None);
        assert!(l.is_done());
        assert_eq!(typed(l.output(b"login: ")), None);
    }

    #[test]
    fn a_user_name_alone_ends_the_login_once_typed() {
        let mut l = AutoLogin::new(Some("admin".into()), None);
        assert_eq!(typed(l.output(b"Username: ")), Some("admin\r".into()));
        assert!(l.is_done());
    }
}

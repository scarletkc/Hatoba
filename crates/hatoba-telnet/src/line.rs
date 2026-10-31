//! The client's own echo and line editing (RFC 857, RFC 858) for a server that leaves them to it,
//! as the `telnet` command and PuTTY do. While the server does not echo, what is typed shows from
//! here; while it does not suppress go-ahead, input goes out a line at a time, edited here with
//! Backspace, Ctrl+U and Ctrl+W. Network devices do both themselves and get every key as typed;
//! a mail server or a bare device console does neither.

/// A line being typed for a server that takes whole lines.
#[derive(Default)]
pub struct LineDiscipline {
    line: String,
    /// The last character was CR, so an LF right after it ends the same line.
    after_cr: bool,
    escape: Escape,
}

/// Where the input is in an escape sequence, such as an arrow key's.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    /// After ESC.
    Start,
    /// In a CSI or SS3 sequence, until its final character.
    Sequence,
}

/// What a piece of the terminal's input becomes.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Typed {
    /// What goes to the server, before Telnet's escaping.
    pub send: Vec<u8>,
    /// What the terminal shows: the client's echo.
    pub echo: Vec<u8>,
}

impl LineDiscipline {
    /// Reads the terminal's input. With `echo`, the server does not echo, so it shows from here;
    /// with `edit`, the server takes whole lines, so it is kept until Enter. Without either, it
    /// goes as it is.
    pub fn input(&mut self, data: &[u8], echo: bool, edit: bool) -> Typed {
        let mut out = Typed::default();
        if !edit {
            out.send.extend_from_slice(data);
            if !echo {
                return out;
            }
        }
        for c in String::from_utf8_lossy(data).chars() {
            // Keys such as the arrows have nothing to move in a line, nor anything to show.
            if self.in_escape(c) {
                continue;
            }
            let after_cr = std::mem::replace(&mut self.after_cr, c == '\r');
            match c {
                '\n' if after_cr => {}
                '\r' | '\n' => {
                    if edit {
                        out.send
                            .append(&mut std::mem::take(&mut self.line).into_bytes());
                        out.send.push(b'\r');
                    }
                    if echo {
                        out.echo.extend_from_slice(b"\r\n");
                    }
                }
                '\x7f' | '\x08' => {
                    let gone = if edit { self.line.pop() } else { Some(' ') };
                    if let (true, Some(gone)) = (echo, gone) {
                        erase(gone, &mut out.echo);
                    }
                }
                // Ctrl+U: the whole line.
                '\x15' if edit => {
                    while let Some(gone) = self.line.pop() {
                        if echo {
                            erase(gone, &mut out.echo);
                        }
                    }
                }
                // Ctrl+W: the last word, and the spaces after it.
                '\x17' if edit => {
                    let word = self.line.trim_end().rfind(' ').map_or(0, |i| i + 1);
                    for gone in self.line.split_off(word).chars().rev() {
                        if echo {
                            erase(gone, &mut out.echo);
                        }
                    }
                }
                // Ctrl+C and the like go at once, as they are.
                c if c.is_control() && c != '\t' => {
                    if edit {
                        let mut buf = [0; 4];
                        out.send
                            .extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                }
                c => {
                    let mut buf = [0; 4];
                    let bytes = c.encode_utf8(&mut buf).as_bytes();
                    if edit {
                        self.line.push(c);
                    }
                    if echo {
                        out.echo.extend_from_slice(bytes);
                    }
                }
            }
        }
        out
    }

    /// The line typed so far, which goes as it is once the server takes input a character at a
    /// time.
    pub fn flush(&mut self) -> Vec<u8> {
        self.escape = Escape::None;
        std::mem::take(&mut self.line).into_bytes()
    }

    /// Whether `c` belongs to an escape sequence.
    fn in_escape(&mut self, c: char) -> bool {
        self.escape = match (self.escape, c) {
            (Escape::None, '\x1b') => Escape::Start,
            (Escape::None, _) => return false,
            (Escape::Start, '[' | 'O') | (Escape::Sequence, '\0'..='?') => Escape::Sequence,
            // Alt with a key, or a sequence's final character.
            _ => Escape::None,
        };
        true
    }
}

/// Takes the character `gone` off the screen: two columns for the wide East Asian scripts and
/// emoji, one for the rest.
fn erase(gone: char, echo: &mut Vec<u8>) {
    let columns = match u32::from(gone) {
        0x1100..=0x115f
        | 0x2e80..=0x303e
        | 0x3041..=0x33ff
        | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff
        | 0xa000..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe30..=0xfe4f
        | 0xff00..=0xff60
        | 0xffe0..=0xffe6
        | 0x1f300..=0x1f64f
        | 0x1f900..=0x1f9ff
        | 0x20000..=0x3fffd => 2,
        _ => 1,
    };
    for _ in 0..columns {
        echo.extend_from_slice(b"\x08 \x08");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(send: &[u8], echo: &[u8]) -> Typed {
        Typed {
            send: send.to_vec(),
            echo: echo.to_vec(),
        }
    }

    #[test]
    fn a_line_is_kept_and_shown_until_enter() {
        let mut l = LineDiscipline::default();
        assert_eq!(l.input(b"EHLX", true, true), typed(b"", b"EHLX"));
        assert_eq!(
            l.input(b"\x7fO x\r", true, true),
            typed(b"EHLO x\r", b"\x08 \x08O x\r\n")
        );
        // A pasted CR LF ends one line.
        assert_eq!(
            l.input(b"a\r\nb\r\n", true, true),
            typed(b"a\rb\r", b"a\r\nb\r\n")
        );
    }

    #[test]
    fn ctrl_u_and_ctrl_w_take_the_line_and_a_word() {
        let mut l = LineDiscipline::default();
        l.input(b"one two  ", true, true);
        assert_eq!(l.input(b"\x17", true, true).echo, b"\x08 \x08".repeat(5));
        assert_eq!(l.input(b"\x15", true, true).echo, b"\x08 \x08".repeat(4));
        assert_eq!(l.input(b"\r", true, true).send, b"\r");
    }

    #[test]
    fn a_wide_character_takes_two_columns_off() {
        let mut l = LineDiscipline::default();
        l.input("交换".as_bytes(), true, true);
        assert_eq!(l.input(b"\x7f", true, true).echo, b"\x08 \x08\x08 \x08");
        assert_eq!(l.input(b"\r", true, true).send, "交\r".as_bytes());
    }

    #[test]
    fn arrow_keys_do_nothing_in_a_line_and_ctrl_c_goes_at_once() {
        let mut l = LineDiscipline::default();
        assert_eq!(
            l.input(b"ab\x1b[D\x1bOA\x1b[1;5Cc", true, true),
            typed(b"", b"abc")
        );
        assert_eq!(l.input(b"\x03", true, true), typed(b"\x03", b""));
        assert_eq!(l.input(b"\r", false, true), typed(b"abc\r", b""));
    }

    #[test]
    fn a_server_that_echoes_but_takes_lines_shows_nothing_from_here() {
        let mut l = LineDiscipline::default();
        assert_eq!(l.input(b"secret\r", false, true), typed(b"secret\r", b""));
    }

    #[test]
    fn a_character_at_a_time_goes_at_once_and_shows_when_the_server_does_not_echo() {
        let mut l = LineDiscipline::default();
        assert_eq!(
            l.input(b"ls\x7f\x1b[A\r", true, false),
            typed(b"ls\x7f\x1b[A\r", b"ls\x08 \x08\r\n")
        );
        assert_eq!(
            l.input(b"\xff\x1b[A", false, false),
            typed(b"\xff\x1b[A", b"")
        );
    }

    #[test]
    fn what_was_typed_goes_once_the_server_takes_characters() {
        let mut l = LineDiscipline::default();
        l.input(b"sh", true, true);
        assert_eq!(l.flush(), b"sh");
        assert_eq!(l.flush(), b"");
    }
}

//! The Telnet protocol (RFC 854) between the terminal and the server (TELNET-02): commands are
//! taken out of the server's output and answered, and the terminal's input is escaped.
//!
//! The client offers the window size (NAWS, RFC 1073) and the terminal type (RFC 1091), asks the
//! server to echo and to suppress go-ahead, and takes binary transmission (RFC 856) either way
//! when the server offers it, so UTF-8 passes unchanged. Every other option is refused. An option
//! is answered only when its state changes, so negotiation never loops (RFC 854, "Option
//! negotiation"). Until the server agrees to binary, the terminal's Enter goes as CR LF, a CR the
//! server sends as CR NUL arrives as CR, and the rest passes as it is.

/// Interpret as command.
pub(crate) const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const BINARY: u8 = 0;
const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;

/// TTYPE subnegotiation: the server asks for the type…
const TTYPE_SEND: u8 = 1;
/// … and the client answers with one.
const TTYPE_IS: u8 = 0;

/// The terminal types offered, most capable first. A server that asks again gets the next one,
/// and the last one again once the list is done (RFC 1091).
const TERMINAL_TYPES: [&str; 3] = ["XTERM-256COLOR", "XTERM", "VT100"];

/// The longest subnegotiation kept; the rest of a longer one is dropped.
const MAX_SUBNEGOTIATION: usize = 512;

/// One option's state on one side (RFC 1143's "Q method", without its queue: the client never
/// changes its mind about an option while a request is open).
#[derive(Clone, Copy, Default)]
struct Side {
    enabled: bool,
    /// The client asked for a change and awaits the answer, which is then not answered again.
    asked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Iac,
    /// After WILL, WONT, DO or DONT, waiting for the option.
    Option(u8),
    /// In a subnegotiation, after `IAC SB`, waiting for the option.
    SbOption,
    Sb,
    SbIac,
}

/// What [`Telnet::receive`] found in a chunk of the server's output.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Received {
    /// The terminal output, without the Telnet commands.
    pub data: Vec<u8>,
    /// What to send back to the server: the answers to its commands.
    pub reply: Vec<u8>,
}

/// The client side of a Telnet connection's protocol state.
pub struct Telnet {
    state: State,
    /// The option being subnegotiated, and what it has sent so far.
    sb_option: u8,
    sb: Vec<u8>,
    /// Options the client performs (the server said DO), by option code.
    us: [Side; 256],
    /// Options the server performs (the server said WILL).
    him: [Side; 256],
    /// The window size, sent whenever NAWS is on.
    cols: u16,
    rows: u16,
    terminal_type: usize,
    /// The last output byte was CR, so a NUL after it is padding.
    after_cr: bool,
    /// The last input byte went as CR LF, so an LF right after it is already sent.
    sent_crlf: bool,
}

impl Telnet {
    /// The protocol state for a new connection with a window of `cols` × `rows`.
    #[must_use]
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            state: State::Data,
            sb_option: 0,
            sb: Vec::new(),
            us: [Side::default(); 256],
            him: [Side::default(); 256],
            cols,
            rows,
            terminal_type: 0,
            after_cr: false,
            sent_crlf: false,
        }
    }

    /// What the client sends first on the Telnet port: it offers the window size and the terminal
    /// type, and asks the server to echo and to suppress go-ahead, as PuTTY does. On another port
    /// the client waits for the server to start, as the `telnet` command does, so a service such
    /// as SMTP gets no Telnet commands.
    pub fn start(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for option in [NAWS, TTYPE, SGA] {
            self.us[usize::from(option)].asked = true;
            out.extend_from_slice(&[IAC, WILL, option]);
        }
        for option in [ECHO, SGA] {
            self.him[usize::from(option)].asked = true;
            out.extend_from_slice(&[IAC, DO, option]);
        }
        out
    }

    /// Takes the Telnet commands out of a chunk of the server's output. Commands may be split
    /// across chunks.
    pub fn receive(&mut self, input: &[u8]) -> Received {
        let mut out = Received::default();
        for &byte in input {
            match self.state {
                State::Data => match byte {
                    IAC => self.state = State::Iac,
                    0 if self.after_cr && !self.him[usize::from(BINARY)].enabled => {
                        self.after_cr = false;
                    }
                    _ => {
                        self.after_cr = byte == b'\r';
                        out.data.push(byte);
                    }
                },
                State::Iac => {
                    self.state = State::Data;
                    match byte {
                        IAC => {
                            self.after_cr = false;
                            out.data.push(IAC);
                        }
                        WILL | WONT | DO | DONT => self.state = State::Option(byte),
                        SB => self.state = State::SbOption,
                        // NOP, data mark, break, go-ahead and the rest carry nothing for a
                        // terminal.
                        _ => {}
                    }
                }
                State::Option(command) => {
                    self.state = State::Data;
                    self.negotiate(command, byte, &mut out.reply);
                }
                State::SbOption => {
                    self.sb_option = byte;
                    self.sb.clear();
                    self.state = State::Sb;
                }
                State::Sb => match byte {
                    IAC => self.state = State::SbIac,
                    _ if self.sb.len() < MAX_SUBNEGOTIATION => self.sb.push(byte),
                    _ => {}
                },
                State::SbIac => match byte {
                    SE => {
                        self.state = State::Data;
                        self.subnegotiate(&mut out.reply);
                    }
                    IAC => {
                        self.state = State::Sb;
                        if self.sb.len() < MAX_SUBNEGOTIATION {
                            self.sb.push(IAC);
                        }
                    }
                    // A server that never sends SE: the subnegotiation ends here.
                    _ => self.state = State::Data,
                },
            }
        }
        out
    }

    /// The terminal's input as it goes to the server: IAC doubled, and Enter as CR LF until the
    /// client sends binary.
    pub fn encode(&mut self, input: &[u8]) -> Vec<u8> {
        let binary = self.us[usize::from(BINARY)].enabled;
        let mut out = Vec::with_capacity(input.len() + 2);
        for &byte in input {
            let sent_crlf = std::mem::take(&mut self.sent_crlf);
            match byte {
                IAC => out.extend_from_slice(&[IAC, IAC]),
                b'\r' if !binary => {
                    out.extend_from_slice(b"\r\n");
                    self.sent_crlf = true;
                }
                // Once binary is on, every byte goes as typed.
                b'\n' if sent_crlf && !binary => {}
                _ => out.push(byte),
            }
        }
        out
    }

    /// Records the window's new size, and returns what tells the server when NAWS is on.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Vec<u8> {
        if (cols, rows) == (self.cols, self.rows) {
            return Vec::new();
        }
        self.cols = cols;
        self.rows = rows;
        if self.us[usize::from(NAWS)].enabled {
            self.window_size()
        } else {
            Vec::new()
        }
    }

    /// Whether the server echoes what is typed. Until it does, the client echoes it
    /// ([`crate::line`]).
    #[must_use]
    pub fn server_echoes(&self) -> bool {
        self.him[usize::from(ECHO)].enabled
    }

    /// Whether the server suppresses go-ahead, taking input a character at a time. Until it does,
    /// the client sends a line at a time ([`crate::line`]).
    #[must_use]
    pub fn server_suppresses_go_ahead(&self) -> bool {
        self.him[usize::from(SGA)].enabled
    }

    fn negotiate(&mut self, command: u8, option: u8, reply: &mut Vec<u8>) {
        let index = usize::from(option);
        match command {
            WILL | WONT => {
                let accept = command == WILL && matches!(option, BINARY | ECHO | SGA);
                let side = &mut self.him[index];
                let asked = std::mem::take(&mut side.asked);
                if side.enabled == accept {
                    // Already so: an answer to a request, or a repeat, which is not answered.
                    if command == WILL && !accept && !asked {
                        // Refused before; a server may offer again, and is refused again.
                        reply.extend_from_slice(&[IAC, DONT, option]);
                    }
                    return;
                }
                side.enabled = accept;
                if !asked {
                    reply.extend_from_slice(&[IAC, if accept { DO } else { DONT }, option]);
                }
            }
            DO | DONT => {
                let accept = command == DO && matches!(option, BINARY | SGA | TTYPE | NAWS);
                let side = &mut self.us[index];
                let asked = std::mem::take(&mut side.asked);
                if side.enabled == accept {
                    if command == DO && !accept && !asked {
                        reply.extend_from_slice(&[IAC, WONT, option]);
                    }
                    return;
                }
                side.enabled = accept;
                if !asked {
                    reply.extend_from_slice(&[IAC, if accept { WILL } else { WONT }, option]);
                }
                if accept && option == NAWS {
                    reply.extend(self.window_size());
                }
            }
            _ => {}
        }
    }

    fn subnegotiate(&mut self, reply: &mut Vec<u8>) {
        if self.sb_option == TTYPE
            && self.sb.first() == Some(&TTYPE_SEND)
            && self.us[usize::from(TTYPE)].enabled
        {
            let name = TERMINAL_TYPES[self.terminal_type.min(TERMINAL_TYPES.len() - 1)];
            self.terminal_type += 1;
            reply.extend_from_slice(&[IAC, SB, TTYPE, TTYPE_IS]);
            reply.extend_from_slice(name.as_bytes());
            reply.extend_from_slice(&[IAC, SE]);
        }
    }

    /// `IAC SB NAWS <cols> <rows> IAC SE`, with a 255 in the size doubled.
    fn window_size(&self) -> Vec<u8> {
        let mut out = vec![IAC, SB, NAWS];
        for byte in self
            .cols
            .to_be_bytes()
            .into_iter()
            .chain(self.rows.to_be_bytes())
        {
            out.push(byte);
            if byte == IAC {
                out.push(IAC);
            }
        }
        out.extend_from_slice(&[IAC, SE]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn telnet() -> Telnet {
        let mut t = Telnet::new(80, 24);
        t.start();
        t
    }

    #[test]
    fn the_client_offers_its_options_first() {
        assert_eq!(
            Telnet::new(80, 24).start(),
            [
                IAC, WILL, NAWS, IAC, WILL, TTYPE, IAC, WILL, SGA, IAC, DO, ECHO, IAC, DO, SGA
            ]
        );
    }

    #[test]
    fn answers_to_the_clients_requests_are_not_answered_again() {
        let mut t = telnet();
        let r = t.receive(&[IAC, WILL, ECHO, IAC, WILL, SGA, IAC, DO, SGA]);
        assert!(r.reply.is_empty());
        assert!(r.data.is_empty());
        assert!(t.server_echoes());
    }

    #[test]
    fn naws_sends_the_window_size_and_its_changes() {
        let mut t = telnet();
        assert_eq!(
            t.receive(&[IAC, DO, NAWS]).reply,
            [IAC, SB, NAWS, 0, 80, 0, 24, IAC, SE]
        );
        assert_eq!(
            t.resize(255, 24),
            [IAC, SB, NAWS, 0, 255, 255, 0, 24, IAC, SE]
        );
        assert!(t.resize(255, 24).is_empty());
    }

    #[test]
    fn the_window_size_waits_for_naws() {
        let mut t = Telnet::new(80, 24);
        assert!(t.resize(100, 30).is_empty());
        assert_eq!(
            t.receive(&[IAC, DO, NAWS]).reply,
            [IAC, WILL, NAWS, IAC, SB, NAWS, 0, 100, 0, 30, IAC, SE]
        );
    }

    #[test]
    fn terminal_types_go_down_the_list_and_stay_on_the_last() {
        let mut t = telnet();
        t.receive(&[IAC, DO, TTYPE]);
        let send = [IAC, SB, TTYPE, TTYPE_SEND, IAC, SE];
        let answer = |name: &str| {
            let mut v = vec![IAC, SB, TTYPE, TTYPE_IS];
            v.extend_from_slice(name.as_bytes());
            v.extend_from_slice(&[IAC, SE]);
            v
        };
        assert_eq!(t.receive(&send).reply, answer("XTERM-256COLOR"));
        assert_eq!(t.receive(&send).reply, answer("XTERM"));
        assert_eq!(t.receive(&send).reply, answer("VT100"));
        assert_eq!(t.receive(&send).reply, answer("VT100"));
    }

    #[test]
    fn other_options_are_refused_once() {
        let mut t = telnet();
        let r = t.receive(&[IAC, DO, 39, IAC, WILL, 5]);
        assert_eq!(r.reply, [IAC, WONT, 39, IAC, DONT, 5]);
        // A DONT/WONT for what is off is not answered.
        assert!(t.receive(&[IAC, DONT, 39, IAC, WONT, 5]).reply.is_empty());
    }

    #[test]
    fn options_the_server_turns_off_are_acknowledged() {
        let mut t = telnet();
        t.receive(&[IAC, WILL, ECHO]);
        assert_eq!(t.receive(&[IAC, WONT, ECHO]).reply, [IAC, DONT, ECHO]);
        assert!(!t.server_echoes());
        assert!(t.receive(&[IAC, WONT, ECHO]).reply.is_empty());
        // Offered again, it is taken again.
        assert_eq!(t.receive(&[IAC, WILL, ECHO]).reply, [IAC, DO, ECHO]);
    }

    #[test]
    fn commands_split_across_chunks_are_still_read() {
        let mut t = telnet();
        let mut data = Vec::new();
        let mut reply = Vec::new();
        for chunk in [&b"ab"[..], &[IAC], &[DO], &[39, b'c', IAC], &[IAC, b'd']] {
            let r = t.receive(chunk);
            data.extend(r.data);
            reply.extend(r.reply);
        }
        assert_eq!(data, [b'a', b'b', b'c', IAC, b'd']);
        assert_eq!(reply, [IAC, WONT, 39]);
    }

    #[test]
    fn subnegotiations_are_kept_out_of_the_output() {
        let mut t = telnet();
        let r = t.receive(&[b'x', IAC, SB, 70, 1, 2, IAC, IAC, 3, IAC, SE, b'y']);
        assert_eq!(r.data, b"xy");
    }

    #[test]
    fn cr_nul_arrives_as_cr_until_binary() {
        let mut t = telnet();
        assert_eq!(t.receive(b"a\r\0b\r").data, b"a\rb\r");
        assert_eq!(t.receive(b"\0c\0").data, b"c\0");
        t.receive(&[IAC, WILL, BINARY]);
        assert_eq!(t.receive(b"\r\0").data, b"\r\0");
    }

    #[test]
    fn input_doubles_iac_and_sends_enter_as_crlf_until_binary() {
        let mut t = telnet();
        assert_eq!(
            t.encode(&[b'a', IAC, b'\r']),
            [b'a', IAC, IAC, b'\r', b'\n']
        );
        // An LF right after the CR is not sent twice, even in the next chunk.
        assert_eq!(t.encode(b"\nb\n"), b"b\n");
        // Binary turning on between the CR and its LF keeps the LF.
        t.encode(b"\r");
        assert_eq!(t.receive(&[IAC, DO, BINARY]).reply, [IAC, WILL, BINARY]);
        assert_eq!(t.encode(b"\n"), b"\n");
        assert_eq!(t.encode(b"\r"), b"\r");
    }
}

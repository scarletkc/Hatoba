//! A server-sent events parser (the WHATWG event stream format), fed with arbitrary chunks.
//!
//! Lines end with LF, CRLF or CR, also when the CR and LF arrive in different chunks. `data`
//! lines of one event are joined with `\n`, comments (`:`) are skipped, and `id` and `retry` are
//! ignored. Bytes are buffered until a line is complete, so a multi-byte character split across
//! chunks decodes correctly. A line that grows over many chunks is scanned for its end once, not
//! again with every chunk.

use crate::error::AiError;

/// A line longer than this without a line end is refused (a hostile server must not exhaust
/// memory).
const MAX_LINE: usize = 32 * 1024 * 1024;

/// One dispatched event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// The `event:` field, when one was given.
    pub event: Option<String>,
    /// The `data:` lines joined with `\n`.
    pub data: String,
}

/// Incremental parser state.
#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buf: Vec<u8>,
    /// The first `scanned` bytes of `buf` are known to hold no line end, so the next chunk is
    /// scanned from there.
    scanned: usize,
    /// The previous chunk ended with CR: a leading LF in the next one belongs to it.
    pending_cr: bool,
    event: Option<String>,
    data: Option<String>,
}

impl SseParser {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feeds one chunk and appends every completed event to `out`.
    pub(crate) fn push(&mut self, chunk: &[u8], out: &mut Vec<SseEvent>) -> Result<(), AiError> {
        let mut chunk = chunk;
        if self.pending_cr && !chunk.is_empty() {
            self.pending_cr = false;
            if let Some(rest) = chunk.strip_prefix(b"\n") {
                chunk = rest;
            }
        }
        self.buf.extend_from_slice(chunk);
        let mut start = 0;
        let mut scan_from = self.scanned;
        while let Some(rel) = self.buf[scan_from..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
        {
            let end = scan_from + rel;
            let line = String::from_utf8_lossy(&self.buf[start..end]).into_owned();
            let mut next = end + 1;
            if self.buf[end] == b'\r' {
                match self.buf.get(next) {
                    Some(b'\n') => next += 1,
                    Some(_) => {}
                    None => self.pending_cr = true,
                }
            }
            self.line(&line, out);
            start = next;
            scan_from = next;
        }
        self.buf.drain(..start);
        // What is left has no line end in it.
        self.scanned = self.buf.len();
        if self.buf.len() > MAX_LINE {
            return Err(AiError::Protocol("a stream line is too long".into()));
        }
        Ok(())
    }

    /// Ends the stream: a last line without a line end and a last event without a blank line
    /// are still dispatched.
    pub(crate) fn finish(&mut self, out: &mut Vec<SseEvent>) {
        if !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&self.buf).into_owned();
            self.buf.clear();
            self.scanned = 0;
            self.line(&line, out);
        }
        self.dispatch(out);
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => match &mut self.data {
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
                None => self.data = Some(value.to_owned()),
            },
            "event" => self.event = Some(value.to_owned()),
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        let event = self.event.take();
        if let Some(data) = self.data.take() {
            out.push(SseEvent { event, data });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut parser = SseParser::new();
        let mut out = Vec::new();
        for chunk in chunks {
            parser.push(chunk, &mut out).unwrap();
        }
        parser.finish(&mut out);
        out
    }

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    const STREAM: &str = ": comment\r\nevent: message_start\r\ndata: {\"a\":\"日本\"}\r\n\r\n\
        data: line1\ndata:line2\nid: 5\nretry: 10\n\n\
        event: ping\rdata: {}\r\r\
        data: [DONE]\n\n";

    #[test]
    fn parses_fields_line_endings_and_multiline_data() {
        let expected = vec![
            ev(Some("message_start"), "{\"a\":\"日本\"}"),
            ev(None, "line1\nline2"),
            ev(Some("ping"), "{}"),
            ev(None, "[DONE]"),
        ];
        assert_eq!(parse_all(&[STREAM.as_bytes()]), expected);
    }

    #[test]
    fn any_chunking_gives_the_same_events() {
        let bytes = STREAM.as_bytes();
        let whole = parse_all(&[bytes]);
        for split in 0..=bytes.len() {
            let (a, b) = bytes.split_at(split);
            assert_eq!(parse_all(&[a, b]), whole, "split at {split}");
        }
        let singles: Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(parse_all(&singles), whole);
    }

    #[test]
    fn a_long_line_in_many_small_chunks_is_scanned_once() {
        // 1 MiB in single bytes: rescanning the buffered part with every chunk would take
        // minutes (about 5 * 10^11 byte comparisons).
        const LEN: usize = 1024 * 1024;
        let payload = "x".repeat(LEN);
        let stream = format!("data: {payload}\n\n");
        let mut parser = SseParser::new();
        let mut out = Vec::new();
        for (i, byte) in stream.as_bytes().chunks(1).enumerate() {
            parser.push(byte, &mut out).unwrap();
            if i < LEN {
                // Everything buffered has been scanned: the next chunk starts after it.
                assert_eq!(parser.scanned, parser.buf.len());
            }
        }
        parser.finish(&mut out);
        assert_eq!(out, vec![ev(None, &payload)]);
        assert!(parser.buf.is_empty());
        assert_eq!(parser.scanned, 0);
    }

    #[test]
    fn a_long_line_ends_correctly_wherever_the_chunks_split() {
        let long = "y".repeat(10_000);
        let stream = format!("data: {long}\r\ndata: tail\r\n\r\nevent: e\rdata: z\r\r");
        let whole = parse_all(&[stream.as_bytes()]);
        assert_eq!(
            whole,
            vec![ev(None, &format!("{long}\ntail")), ev(Some("e"), "z")]
        );
        // Chunk sizes that put every line end (CR, LF, CRLF) on a chunk boundary somewhere,
        // after a long run of bytes without one.
        for size in [1, 2, 3, 7, 64, 4096, 10_006, 10_007, 10_008] {
            let chunks: Vec<&[u8]> = stream.as_bytes().chunks(size).collect();
            assert_eq!(parse_all(&chunks), whole, "chunks of {size}");
        }
    }

    #[test]
    fn flushes_an_unterminated_last_event() {
        assert_eq!(parse_all(&[b"data: tail"]), vec![ev(None, "tail")]);
        assert_eq!(parse_all(&[b"event: x\n\n"]), Vec::<SseEvent>::new());
        assert_eq!(parse_all(&[b"data\n\n"]), vec![ev(None, "")]);
    }
}

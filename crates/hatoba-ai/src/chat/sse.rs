//! A server-sent events parser (the WHATWG event stream format), fed with arbitrary chunks.
//!
//! Lines end with LF, CRLF or CR, also when the CR and LF arrive in different chunks. `data`
//! lines of one event are joined with `\n`, comments (`:`) are skipped, and `id` and `retry` are
//! ignored. Bytes are buffered until a line is complete, so a multi-byte character split across
//! chunks decodes correctly.

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
        while let Some(rel) = self.buf[start..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
        {
            let end = start + rel;
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
        }
        self.buf.drain(..start);
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
    fn flushes_an_unterminated_last_event() {
        assert_eq!(parse_all(&[b"data: tail"]), vec![ev(None, "tail")]);
        assert_eq!(parse_all(&[b"event: x\n\n"]), Vec::<SseEvent>::new());
        assert_eq!(parse_all(&[b"data\n\n"]), vec![ev(None, "")]);
    }
}

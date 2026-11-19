// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Clean-room W3C Server-Sent Events (`text/event-stream`) incremental parser.
//!
//! Implements the WHATWG HTML `text/event-stream` interpretation rules:
//! leading UTF-8 BOM (`U+FEFF`) stripping on the first line, `\r\n` / `\n` / `\r`
//! line endings across arbitrary chunk boundaries, `:comment` lines, multi-line
//! `data:` concatenation, `event:` type overrides, `id:` persistence (ignoring
//! values containing `U+0000`), and ASCII-digit `retry:` reconnection intervals.

/// Maximum buffered incomplete line or event payload accepted by [`SseParser`] (8 MiB).
pub const MAX_SSE_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// One dispatched Server-Sent Event record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    /// Event type name (`"message"` when no non-empty `event:` field was set).
    pub event_type: String,
    /// Event payload with trailing newline removed after joining `data:` lines.
    pub data: String,
    /// Active `lastEventId` string at the moment of dispatch.
    pub last_event_id: String,
}

/// Incremental `text/event-stream` parser supporting chunked delivery.
#[derive(Clone, Debug)]
pub struct SseParser {
    buffer: Vec<u8>,
    saw_first_chunk: bool,
    pending_cr: bool,
    event_type: String,
    data_buf: String,
    has_data: bool,
    last_event_id: String,
    retry_ms: Option<u64>,
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SseParser {
    /// Create a fresh `text/event-stream` parser with empty `lastEventId`.
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            saw_first_chunk: false,
            pending_cr: false,
            event_type: String::new(),
            data_buf: String::new(),
            has_data: false,
            last_event_id: String::new(),
            retry_ms: None,
        }
    }

    /// Create a parser seeded with `Last-Event-ID` from a prior connection.
    pub fn with_last_event_id(last_event_id: impl Into<String>) -> Self {
        let mut p = Self::new();
        p.last_event_id = last_event_id.into();
        p
    }

    /// Current `lastEventId` value tracked across events.
    pub fn last_event_id(&self) -> &str {
        &self.last_event_id
    }

    /// Most recent valid `retry:` reconnection delay in milliseconds, if received.
    pub fn retry_ms(&self) -> Option<u64> {
        self.retry_ms
    }

    /// Feed a raw byte chunk into the stream and return all complete events dispatched.
    pub fn feed(&mut self, mut chunk: &[u8]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if chunk.is_empty() {
            return out;
        }
        if !self.saw_first_chunk {
            self.saw_first_chunk = true;
            if let Some(stripped) = chunk.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
                chunk = stripped;
            }
        }
        for &b in chunk {
            if self.pending_cr {
                self.pending_cr = false;
                if b == b'\n' {
                    continue;
                }
            }
            match b {
                b'\r' => {
                    self.pending_cr = true;
                    self.consume_complete_line(&mut out);
                }
                b'\n' => {
                    self.consume_complete_line(&mut out);
                }
                _ => {
                    if self.buffer.len() < MAX_SSE_BUFFER_BYTES {
                        self.buffer.push(b);
                    }
                }
            }
        }
        out
    }

    /// Flush end-of-stream state. Per the W3C SSE specification, an unterminated
    /// trailing block without a blank line is discarded rather than dispatched.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        self.pending_cr = false;
        self.buffer.clear();
        self.event_type.clear();
        self.data_buf.clear();
        self.has_data = false;
        Vec::new()
    }

    fn consume_complete_line(&mut self, out: &mut Vec<SseEvent>) {
        let raw_line = std::mem::take(&mut self.buffer);
        let line = String::from_utf8_lossy(&raw_line);
        let line_ref = line.as_ref();
        if line_ref.is_empty() {
            if self.has_data {
                if self.data_buf.ends_with('\n') {
                    self.data_buf.pop();
                }
                let ev_type = if self.event_type.is_empty() {
                    "message".to_string()
                } else {
                    std::mem::take(&mut self.event_type)
                };
                let data = std::mem::take(&mut self.data_buf);
                self.has_data = false;
                out.push(SseEvent {
                    event_type: ev_type,
                    data,
                    last_event_id: self.last_event_id.clone(),
                });
            } else {
                self.event_type.clear();
                self.data_buf.clear();
            }
            return;
        }
        if line_ref.starts_with(':') {
            return;
        }
        let (field, raw_val) = match line_ref.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line_ref, ""),
        };
        match field {
            "event" => {
                self.event_type.clear();
                self.event_type.push_str(raw_val);
            }
            "data" => {
                if self.data_buf.len() + raw_val.len() < MAX_SSE_BUFFER_BYTES {
                    self.data_buf.push_str(raw_val);
                    self.data_buf.push('\n');
                    self.has_data = true;
                }
            }
            "id" => {
                if !raw_val.contains('\0') {
                    self.last_event_id.clear();
                    self.last_event_id.push_str(raw_val);
                }
            }
            "retry" => {
                if !raw_val.is_empty()
                    && raw_val.bytes().all(|b| b.is_ascii_digit())
                    && let Ok(ms) = raw_val.parse::<u64>()
                {
                    self.retry_ms = Some(ms);
                }
            }
            _ => {}
        }
    }
}

/// Parse a complete `text/event-stream` response body, returning
/// `(events, final_last_event_id, retry_ms)`.
pub fn parse_sse_stream(
    bytes: &[u8],
    initial_last_event_id: Option<&str>,
) -> (Vec<SseEvent>, String, Option<u64>) {
    let mut parser = match initial_last_event_id {
        Some(id) => SseParser::with_last_event_id(id),
        None => SseParser::new(),
    };
    let events = parser.feed(bytes);
    let _ = parser.finish();
    (
        events,
        parser.last_event_id().to_string(),
        parser.retry_ms(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bom_comments_multiline_data_custom_events_ids_and_retry() {
        let stream = b"\xEF\xBB\xBF: keep-alive comment\r\nid: 42\r\nretry: 2500\r\nevent: update\r\ndata: first line\r\ndata: second line\r\n\r\ndata: plain message\n\nid: bad\0id\ndata: keeps 42\r\r";
        let (events, last_id, retry_ms) = parse_sse_stream(stream, None);
        assert_eq!(retry_ms, Some(2500));
        assert_eq!(last_id, "42");
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0],
            SseEvent {
                event_type: "update".to_string(),
                data: "first line\nsecond line".to_string(),
                last_event_id: "42".to_string(),
            }
        );
        assert_eq!(
            events[1],
            SseEvent {
                event_type: "message".to_string(),
                data: "plain message".to_string(),
                last_event_id: "42".to_string(),
            }
        );
        assert_eq!(
            events[2],
            SseEvent {
                event_type: "message".to_string(),
                data: "keeps 42".to_string(),
                last_event_id: "42".to_string(),
            }
        );
    }

    #[test]
    fn incremental_chunks_split_across_crlf_and_discard_unterminated_tail() {
        let mut p = SseParser::new();
        assert!(p.feed(b"data: hel").is_empty());
        assert!(p.feed(b"lo\r").is_empty());
        let batch = p.feed(b"\n\r\ndata: unterminated");
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].data, "hello");
        assert!(p.finish().is_empty());
    }
}

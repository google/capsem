//! SSE wire-format parser. Fed chunk-by-chunk, emits complete SSE events.
//!
//! Hand-rolled (no crate). Handles: \r\n/\n/\r line endings, comment lines
//! (`:` prefix), multiple `data:` lines joined with \n, `[DONE]` sentinel
//! filtering, and partial chunks split at arbitrary byte boundaries.
//!
//! Hot path: called for every response body chunk from AI providers.
//! Optimized for minimal allocation: reuses line buffer, avoids intermediate
//! copies, and emits events in-place.

/// A single SSE event parsed from the wire format.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    /// The `event:` field value (e.g., "message_start" for Anthropic).
    /// None if no `event:` line preceded this event's data.
    pub event_type: Option<String>,
    /// The concatenated `data:` field values (multiple `data:` lines joined with \n).
    pub data: String,
}

/// Stateful SSE parser. Feed it byte chunks via `feed()`, get back complete events.
pub struct SseParser {
    line_buf: Vec<u8>,
    current_event_type: Option<String>,
    current_data: Vec<String>,
    last_was_cr: bool,
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            line_buf: Vec::with_capacity(512),
            current_event_type: None,
            current_data: Vec::new(),
            last_was_cr: false,
        }
    }

    /// Feed a chunk of bytes. Returns any complete SSE events found.
    ///
    /// Chunks can be split at arbitrary byte boundaries -- the parser
    /// maintains internal state across calls.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        let mut events = Vec::new();

        for &byte in chunk {
            if self.last_was_cr {
                self.last_was_cr = false;
                if byte == b'\n' {
                    // \r\n pair -- line was already processed on \r
                    continue;
                }
                // Lone \r -- line was processed, now handle this new byte
            }

            if byte == b'\r' {
                self.last_was_cr = true;
                if let Some(event) = self.process_line() {
                    events.push(event);
                }
            } else if byte == b'\n' {
                if let Some(event) = self.process_line() {
                    events.push(event);
                }
            } else {
                self.line_buf.push(byte);
            }
        }

        events
    }

    /// Flush any remaining buffered data as a final event.
    /// Call this when the stream ends to capture any trailing event
    /// that wasn't terminated by an empty line.
    pub fn flush(&mut self) -> Option<SseEvent> {
        if !self.line_buf.is_empty() {
            self.process_line();
        }
        self.dispatch_event()
    }

    /// Process the current line buffer. Returns an event if an empty line
    /// triggers dispatch.
    fn process_line(&mut self) -> Option<SseEvent> {
        let line = std::mem::take(&mut self.line_buf);

        // Empty line: dispatch accumulated event
        if line.is_empty() {
            return self.dispatch_event();
        }

        // Comment line (starts with ':') -- ignore
        if line[0] == b':' {
            return None;
        }

        // Parse "field: value" or "field:value" or just "field"
        let (field, value) = if let Some(pos) = line.iter().position(|&b| b == b':') {
            let field = &line[..pos];
            // SSE spec: if char after ':' is space, skip it
            let value = if pos + 1 < line.len() && line[pos + 1] == b' ' {
                &line[pos + 2..]
            } else {
                &line[pos + 1..]
            };
            (field, value)
        } else {
            (line.as_slice(), &[] as &[u8])
        };

        match field {
            b"event" => {
                self.current_event_type = Some(String::from_utf8_lossy(value).into_owned());
            }
            b"data" => {
                self.current_data
                    .push(String::from_utf8_lossy(value).into_owned());
            }
            // id, retry, etc. -- not needed for LLM SSE parsing
            _ => {}
        }

        None
    }

    /// Dispatch the accumulated event data. Returns None if no data was accumulated.
    fn dispatch_event(&mut self) -> Option<SseEvent> {
        if self.current_data.is_empty() && self.current_event_type.is_none() {
            return None;
        }

        let data = self.current_data.join("\n");
        let event_type = self.current_event_type.take();
        self.current_data.clear();

        // Filter [DONE] sentinel (OpenAI convention)
        if data == "[DONE]" {
            return None;
        }

        Some(SseEvent { event_type, data })
    }
}

#[cfg(test)]
mod tests;

use super::*;

// ── Basic parsing ───────────────────────────────────────────────

#[test]
fn simple_data_event() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: hello world\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "hello world");
    assert_eq!(events[0].event_type, None);
}

#[test]
fn event_type_and_data() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type.as_deref(), Some("message_start"));
    assert_eq!(events[0].data, "{\"type\":\"message_start\"}");
}

#[test]
fn multiple_events_in_one_chunk() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: first\n\ndata: second\n\n");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].data, "first");
    assert_eq!(events[1].data, "second");
}

#[test]
fn multiple_data_lines_joined_with_newline() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: line1\ndata: line2\ndata: line3\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "line1\nline2\nline3");
}

#[test]
fn data_without_space_after_colon() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data:no-space\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "no-space");
}

// ── Line endings ────────────────────────────────────────────────

#[test]
fn crlf_line_endings() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: hello\r\n\r\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "hello");
}

#[test]
fn bare_cr_line_endings() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: hello\r\r");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "hello");
}

#[test]
fn mixed_line_endings() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"event: test\r\ndata: mixed\n\r\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type.as_deref(), Some("test"));
    assert_eq!(events[0].data, "mixed");
}

// ── Chunk splitting ─────────────────────────────────────────────

#[test]
fn split_across_two_chunks() {
    let mut parser = SseParser::new();
    let e1 = parser.feed(b"data: hel");
    assert!(e1.is_empty());
    let e2 = parser.feed(b"lo\n\n");
    assert_eq!(e2.len(), 1);
    assert_eq!(e2[0].data, "hello");
}

#[test]
fn split_at_crlf_boundary() {
    let mut parser = SseParser::new();
    let e1 = parser.feed(b"data: test\r");
    assert!(e1.is_empty());
    let e2 = parser.feed(b"\n\r\n");
    // After \r, line is processed. Then \n is skip (crlf pair).
    // Then \r\n is the empty line dispatch.
    assert_eq!(e2.len(), 1);
    assert_eq!(e2[0].data, "test");
}

#[test]
fn split_mid_field_name() {
    let mut parser = SseParser::new();
    let e1 = parser.feed(b"ev");
    assert!(e1.is_empty());
    let e2 = parser.feed(b"ent: ping\ndata: pong\n\n");
    assert_eq!(e2.len(), 1);
    assert_eq!(e2[0].event_type.as_deref(), Some("ping"));
    assert_eq!(e2[0].data, "pong");
}

#[test]
fn many_tiny_chunks() {
    let mut parser = SseParser::new();
    let input = b"data: hello\n\n";
    let mut all_events = Vec::new();
    for &byte in input {
        all_events.extend(parser.feed(&[byte]));
    }
    assert_eq!(all_events.len(), 1);
    assert_eq!(all_events[0].data, "hello");
}

// ── Comments ────────────────────────────────────────────────────

#[test]
fn comment_lines_ignored() {
    let mut parser = SseParser::new();
    let events = parser.feed(b": this is a comment\ndata: real data\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "real data");
}

#[test]
fn comment_between_events() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: first\n\n: heartbeat\n\ndata: second\n\n");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].data, "first");
    assert_eq!(events[1].data, "second");
}

// ── [DONE] sentinel ─────────────────────────────────────────────

#[test]
fn done_sentinel_filtered() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: real\n\ndata: [DONE]\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "real");
}

#[test]
fn done_sentinel_not_partial_match() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: [DONE]x\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "[DONE]x"); // not filtered
}

// ── Flush ───────────────────────────────────────────────────────

#[test]
fn flush_trailing_event() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: no-trailing-newline");
    assert!(events.is_empty());
    let flushed = parser.flush();
    assert_eq!(flushed.unwrap().data, "no-trailing-newline");
}

#[test]
fn flush_empty_parser() {
    let mut parser = SseParser::new();
    assert!(parser.flush().is_none());
}

#[test]
fn flush_after_complete_events() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: complete\n\n");
    assert_eq!(events.len(), 1);
    assert!(parser.flush().is_none());
}

// ── Empty data ──────────────────────────────────────────────────

#[test]
fn empty_data_field() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data:\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "");
}

#[test]
fn data_with_space_only() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: \n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "");
}

// ── Event type resets between events ─────────────────────────────

#[test]
fn event_type_does_not_carry_over() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"event: typed\ndata: first\n\ndata: second\n\n");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type.as_deref(), Some("typed"));
    assert_eq!(events[1].event_type, None);
}

// ── Adversarial inputs ──────────────────────────────────────────

#[test]
fn extremely_long_line() {
    let mut parser = SseParser::new();
    let long_value = "x".repeat(100_000);
    let input = format!("data: {long_value}\n\n");
    let events = parser.feed(input.as_bytes());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.len(), 100_000);
}

#[test]
fn garbage_bytes_mid_stream() {
    let mut parser = SseParser::new();
    // Feed valid event, then garbage, then another valid event
    let events = parser.feed(b"data: good\n\n");
    assert_eq!(events.len(), 1);
    let events = parser.feed(&[0xFF, 0xFE, 0x00, b'\n', b'\n']);
    // Garbage line processed as unknown field, empty line dispatches nothing
    // (no data: field was set)
    assert!(events.is_empty());
    let events = parser.feed(b"data: still good\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "still good");
}

#[test]
fn consecutive_empty_lines() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: one\n\n\n\n\ndata: two\n\n");
    // First empty line dispatches "one". Subsequent empty lines
    // dispatch nothing (no accumulated data).
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].data, "one");
    assert_eq!(events[1].data, "two");
}

#[test]
fn data_with_colons_in_value() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"data: {\"key\": \"value:with:colons\"}\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "{\"key\": \"value:with:colons\"}");
}

#[test]
fn unknown_fields_ignored() {
    let mut parser = SseParser::new();
    let events = parser.feed(b"id: 123\nretry: 5000\ndata: kept\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "kept");
}

// ── Real-world Anthropic SSE ────────────────────────────────────

#[test]
fn anthropic_stream_sample() {
    let mut parser = SseParser::new();
    let input = b"\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"model\":\"claude-sonnet-4-20250514\"}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

    let events = parser.feed(input);
    assert_eq!(events.len(), 6);
    assert_eq!(events[0].event_type.as_deref(), Some("message_start"));
    assert_eq!(events[1].event_type.as_deref(), Some("content_block_start"));
    assert_eq!(events[2].event_type.as_deref(), Some("content_block_delta"));
    assert_eq!(events[3].event_type.as_deref(), Some("content_block_stop"));
    assert_eq!(events[4].event_type.as_deref(), Some("message_delta"));
    assert_eq!(events[5].event_type.as_deref(), Some("message_stop"));
}

// ── Real-world OpenAI SSE ───────────────────────────────────────

#[test]
fn openai_stream_sample() {
    let mut parser = SseParser::new();
    let input = b"\
data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\
\n\
data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\
\n\
data: [DONE]\n\
\n";

    let events = parser.feed(input);
    assert_eq!(events.len(), 2); // [DONE] filtered
    assert_eq!(events[0].event_type, None);
    assert!(events[0].data.contains("chatcmpl-1"));
}

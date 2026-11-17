use super::*;
use std::io::{BufReader, Cursor};

fn messages(input: &[u8], capacity: usize, limit: usize) -> Vec<Result<Vec<u8>, ()>> {
    let mut lines = McpLines::new(BufReader::with_capacity(capacity, Cursor::new(input)), limit);
    let mut result = Vec::new();
    loop {
        match lines.next_line().unwrap() {
            LineRead::Message(bytes) => {
                assert!(bytes.len() <= limit);
                assert!(bytes.capacity() <= limit + 1);
                result.push(Ok(bytes));
            }
            LineRead::TooLarge => result.push(Err(())),
            LineRead::End => return result,
        }
    }
}

#[test]
fn exact_bounds_preserve_crlf_utf8_empty_lines_and_eof_across_every_split() {
    for capacity in 1..=24 {
        assert_eq!(
            messages("éé\r\n\nnext\nlast".as_bytes(), capacity, 4),
            vec![
                Ok("éé".as_bytes().to_vec()),
                Ok(vec![]),
                Ok(b"next".to_vec()),
                Ok(b"last".to_vec())
            ]
        );
        assert_eq!(messages(b"abc\r", capacity, 4), vec![Ok(b"abc\r".to_vec())]);
        assert!(messages(b"", capacity, 4).is_empty());
    }
}

#[test]
fn overflow_is_refused_once_and_next_line_survives_every_split() {
    for capacity in 1..=24 {
        for input in [&b"abcde\nnext\n"[..], &b"abcde\r\nnext\n"[..]] {
            assert_eq!(messages(input, capacity, 4), vec![Err(()), Ok(b"next".to_vec())]);
        }
        assert_eq!(messages(b"abcd\r", capacity, 4), vec![Err(())]);
        assert_eq!(messages(b"abcde", capacity, 4), vec![Err(())]);
    }
}

#[test]
fn real_payload_ceiling_is_accepted_without_truncation() {
    let limit = capsem_proto::MCP_PAYLOAD_MAX_BYTES;
    let mut input = vec![b'x'; limit];
    input.extend_from_slice(b"\r\n");
    assert_eq!(messages(&input, 8192, limit), vec![Ok(vec![b'x'; limit])]);
}

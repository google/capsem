use super::*;

#[test]
fn every_byte_is_forwarded_without_decoding_or_replacement() {
    let input = (0..=u8::MAX).cycle().take(4096).collect::<Vec<_>>();
    for size in 1..=64 {
        let mut boundary = Utf8Boundary::default();
        let mut output = Vec::new();
        for chunk in input.chunks(size) {
            boundary.process(chunk, |bytes| output.extend_from_slice(bytes));
            assert!(boundary.len <= 3);
        }
        assert_eq!(output, input);
    }
}

#[test]
fn ordinary_batches_are_forwarded_in_one_call() {
    for input in [b"ordinary\x1b[31moutput".as_slice(), "漢字 e\u{301} 🦀".as_bytes()] {
        let mut boundary = Utf8Boundary::default();
        let mut calls = Vec::new();
        boundary.process(input, |bytes| calls.push(bytes.to_vec()));
        assert_eq!(calls, [input]);
        assert_eq!(boundary.len, 0);
    }
}

#[test]
fn only_a_fragmented_scalar_is_forwarded_one_byte_at_a_time() {
    let mut boundary = Utf8Boundary::default();
    boundary.process(b"e\xcc", |_| {});
    assert_eq!(boundary.len, 1);
    let mut calls = Vec::new();
    boundary.process(b"\x81 \xf0\x9f\xa6\x80", |bytes| calls.push(bytes.to_vec()));
    // Complete the original combining mark, then batch all remaining bytes.
    assert_eq!(calls, [vec![0x81], " 🦀".as_bytes().to_vec()]);
}

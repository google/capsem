use super::*;

#[test]
fn incomplete_osc_storage_stays_bounded_and_recovers_at_all_terminators() {
    for terminator in [b"\x07".as_slice(), b"\x18", b"\x1a", b"\x1b\\"] {
        let mut budget = OscBudget::default();
        let mut forwarded = Vec::new();
        budget.process(b"\x1b]2;", |bytes| forwarded.extend_from_slice(bytes));
        for _ in 0..1024 {
            budget.process(&[b'x'; 4096], |bytes| forwarded.extend_from_slice(bytes));
            assert!(budget.pending.len() <= TERMINAL_CONTROL_STRING_BYTES);
            assert!(budget.pending.capacity() <= TERMINAL_CONTROL_STRING_BYTES * 2);
        }
        budget.process(terminator, |bytes| forwarded.extend_from_slice(bytes));
        budget.process(b"visible\x1b]2;Ready\x07", |bytes| forwarded.extend_from_slice(bytes));
        assert!(forwarded.len() < 100);
        assert!(forwarded.ends_with(b"visible\x1b]2;Ready\x07"));
    }
}

#[test]
fn fragmented_normal_sequences_pass_through_exactly() {
    let input = b"normal\x1b[31mred\x1b[0m\x1b]2;title\x07\x1b]9;ready\x1b\\\x1b]2;cancel\x18\x1b\x07]2;bell\x07";
    for chunk_size in 1..=input.len() {
        let mut budget = OscBudget::default();
        let mut forwarded = Vec::new();
        for bytes in input.chunks(chunk_size) {
            budget.process(bytes, |bytes| forwarded.extend_from_slice(bytes));
        }
        assert_eq!(forwarded, input);
    }
}

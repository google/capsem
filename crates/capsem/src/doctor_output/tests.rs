use super::*;

/// `capsem doctor` scans a rolling tail of lossily decoded terminal output for
/// its RESULT sentinel. The tail was trimmed with a byte index, which panics
/// when it lands inside a multi-byte character such as the U+FFFD that lossy
/// decoding produces for invalid output.
#[test]
fn doctor_output_tail_trims_on_a_char_boundary() {
    let mut tail = String::new();
    for _ in 0..400 {
        push_doctor_output_tail(&mut tail, &[0xff, b'x']);
    }
    push_doctor_output_tail(&mut tail, "é".repeat(700).as_bytes());
    assert!(tail.len() <= DOCTOR_OUTPUT_TAIL_BYTES + 3);
    push_doctor_output_tail(&mut tail, b"RESULT: FAIL");
    assert!(tail.ends_with("RESULT: FAIL"));
}

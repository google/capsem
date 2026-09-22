//! The rolling tail `capsem doctor` scans for its RESULT sentinel.

/// Bytes of decoded doctor output kept for sentinel matching. Padded by the
/// sentinel length so "RESULT: FAIL" is never split across a trim.
pub(crate) const DOCTOR_OUTPUT_TAIL_BYTES: usize = 512 + "RESULT: FAIL".len();

/// Append lossily decoded terminal output to the doctor's sentinel tail and
/// trim it to roughly `DOCTOR_OUTPUT_TAIL_BYTES`, always on a char boundary.
pub(crate) fn push_doctor_output_tail(tail: &mut String, data: &[u8]) {
    tail.push_str(&String::from_utf8_lossy(data));
    if tail.len() > 2 * DOCTOR_OUTPUT_TAIL_BYTES {
        let mut start = tail.len() - DOCTOR_OUTPUT_TAIL_BYTES;
        while !tail.is_char_boundary(start) {
            start += 1;
        }
        tail.drain(..start);
    }
}

#[cfg(test)]
mod tests;

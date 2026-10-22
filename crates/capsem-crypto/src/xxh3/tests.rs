use super::*;

#[test]
fn empty_input_matches_the_xxh3_128_reference_vector() {
    assert_eq!(hash(b""), 0x99aa06d3014798d86001c324468d497f_u128);
    assert_eq!(Hasher::new().finalize(), hash(b""));
}

#[test]
fn streaming_matches_one_shot_across_simd_and_chunk_boundaries() {
    let bytes: Vec<_> = (0..65_537).map(|index| (index % 251) as u8).collect();
    for size in [1, 16, 17, 32, 64, 128, 129, 240, 241, 1024, 65_536, 65_537] {
        for chunk in [1, 7, 64, 1024, 8192] {
            let mut state = Hasher::new();
            for part in bytes[..size].chunks(chunk) {
                state.update(part);
            }
            assert_eq!(state.finalize(), hash(&bytes[..size]), "size {size}, chunk {chunk}");
        }
    }
}

#[test]
fn standard_writer_hashes_the_existing_bytes_without_a_document_buffer() {
    use std::io::Write;
    let mut writer = Hasher::new();
    writer.write_all(b"first").unwrap();
    writer.write_all(b"-second").unwrap();
    writer.flush().unwrap();
    assert_eq!(writer.finalize(), hash(b"first-second"));
}

#[test]
fn cloned_state_can_continue_independently() {
    let mut original = Hasher::new();
    original.update(b"prefix");
    let mut fork = original.clone();
    fork.update(b"-fork");
    original.update(b"-original");
    assert_eq!(fork.finalize(), hash(b"prefix-fork"));
    assert_eq!(original.finalize(), hash(b"prefix-original"));
}

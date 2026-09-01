use super::*;
use std::io::Write;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(bytes).unwrap();
    enc.finish().unwrap()
}

#[test]
fn decompresses_within_cap() {
    let payload = vec![b'a'; 4096];
    let compressed = gzip(&payload);
    let out = decompress_gzip_capped(&compressed, 1024 * 1024).unwrap();
    assert_eq!(out, payload);
}

#[test]
fn rejects_body_exceeding_cap() {
    // Highly compressible payload: 1 MiB of zeros gzips to a few hundred bytes
    // but decompresses past a small cap. A compression bomb must be refused,
    // not read into host memory.
    let payload = vec![0u8; 1024 * 1024];
    let compressed = gzip(&payload);
    assert!(compressed.len() < 4096, "sanity: bomb is small compressed");
    let err = decompress_gzip_capped(&compressed, 64 * 1024).unwrap_err();
    assert!(err.to_string().contains("cap"), "error must name the cap, got: {err}");
}

#[test]
fn cap_bounds_allocation_not_just_output() {
    // Even at the boundary, the reader must not allocate far beyond the cap:
    // a body one byte over the cap is rejected.
    let payload = vec![0u8; 200_000];
    let compressed = gzip(&payload);
    assert!(decompress_gzip_capped(&compressed, 200_000).is_ok());
    assert!(decompress_gzip_capped(&compressed, 199_999).is_err());
}

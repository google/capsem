use super::*;
use sha2::{Digest as _, Sha256 as Reference};

#[test]
fn accelerated_sha256_preserves_standard_vectors() {
    assert_eq!(
        format!("{:x}", Sha256::default().finalize()),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    for (bytes, expected) in [
        (
            b"".as_slice(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc".as_slice(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
    ] {
        let digest = Sha256::digest(bytes);
        assert_eq!(format!("{digest:x}"), expected);
        assert_eq!(&*digest, digest.as_ref());
    }
}

#[test]
fn accelerated_streaming_sha256_matches_independent_backend_at_block_and_io_boundaries() {
    for length in [1, 55, 56, 63, 64, 65, 127, 128, 129, 65535, 65536, 65537, 1048577] {
        let bytes: Vec<_> = (0..length).map(|index| (index % 251) as u8).collect();
        let expected = Reference::digest(&bytes);
        for chunk in [1, 63, 64, 65, 65536] {
            let mut digest = Sha256::new();
            for part in bytes.chunks(chunk) {
                digest.update(part);
            }
            let actual = digest.finalize();
            assert_eq!(actual.as_ref(), expected.as_slice(), "length={length}, chunk={chunk}");
            assert_eq!(format!("{actual:x}"), format!("{expected:x}"));
        }
    }
}

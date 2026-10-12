use super::*;

#[test]
fn reader_failure_propagates_without_accepting_partial_content() {
    struct Broken;
    impl io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
    }
    let expected = format!("blake3:{}", crate::blake3::hash(b"").to_hex());
    assert_eq!(
        blake3(Broken, &expected, 0).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(blake3(b"".as_slice(), &expected, 0).unwrap(), 0);
}

#[test]
fn the_standard_vector_verifies_and_corruption_truncation_and_growth_fail() {
    let expected = "blake3:6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";
    assert_eq!(blake3(b"abc".as_slice(), expected, 3).unwrap(), 3);
    for content in [b"abd".as_slice(), b"ab".as_slice(), b"abcd".as_slice()] {
        assert_eq!(
            blake3(content, expected, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[test]
fn malformed_algorithm_case_length_and_path_shaped_digests_are_refused() {
    for digest in ["sha256:abc", "blake3:../root", "blake3:ABC", "blake3:"] {
        assert_eq!(
            blake3(b"abc".as_slice(), digest, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

#[test]
fn excess_input_is_bounded_to_the_first_excess_byte() {
    let expected = format!("blake3:{}", crate::blake3::hash(b"abc").to_hex());
    let mut input = io::Cursor::new(vec![0; 8 * 1024 * 1024]);
    assert!(blake3(&mut input, &expected, 3).is_err());
    assert_eq!(input.position(), 4);
}

#[test]
fn streaming_verifier_uses_the_declared_identity_and_refuses_unknown_algorithms() {
    for expected in [
        "blake3:6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
        #[cfg(feature = "sha256")]
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    ] {
        let mut verifier = Verifier::new(expected).unwrap();
        verifier.update(b"a");
        verifier.update(b"bc");
        assert!(verifier.finish());
        let mut wrong = Verifier::new(expected).unwrap();
        wrong.update(b"abd");
        assert!(!wrong.finish());
    }
    for expected in ["sha512:aaaa", "blake3:abcd", "blake3:../root"] {
        assert!(Verifier::new(expected).is_err());
    }
}

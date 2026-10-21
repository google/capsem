use super::*;

#[test]
fn data_is_a_named_messagepack_binary_field_and_decodes_without_copying() {
    let bytes = [0, 255, b'\n', 0x1b, b'['];
    let encoded = encode(&Message::Stdin(&bytes)).unwrap();
    let mut expected = b"\x81\xa5stdin\xc4\x05".to_vec();
    expected.extend_from_slice(&bytes);
    assert_eq!(encoded, expected);
    let Message::Stdin(decoded) = decode_client(&encoded).unwrap() else {
        panic!("not stdin")
    };
    assert_eq!(decoded, bytes);
    assert!(decoded.as_ptr() >= encoded.as_ptr());
    assert!(decoded.as_ptr() < encoded.as_ptr().wrapping_add(encoded.len()));
}

#[test]
fn metadata_is_named_and_each_side_accepts_only_its_messages() {
    let control = StreamControl::Focus { cols: 101, rows: 33 };
    let encoded = encode(&Message::Control(Cow::Borrowed(&control))).unwrap();
    assert_eq!(decode_client(&encoded).unwrap(), Message::Control(Cow::Owned(control)));
    assert!(decode_server(&encoded).is_err());
    let status = StreamStatus::Exit {
        code: 7,
        truncated: false,
    };
    let encoded = encode(&Message::Status(Cow::Borrowed(&status))).unwrap();
    assert_eq!(decode_server(&encoded).unwrap(), Message::Status(Cow::Owned(status)));
    assert!(decode_client(&encoded).is_err());
    for message in [Message::Stdout(b"out"), Message::Stderr(b"err")] {
        let encoded = encode(&message).unwrap();
        assert_eq!(decode_server(&encoded).unwrap(), message);
        assert!(decode_client(&encoded).is_err());
    }
}

#[test]
fn invalid_dimensions_directions_lengths_and_old_wire_formats_are_refused() {
    let invalid = Message::Control(Cow::Owned(StreamControl::Resize { cols: 0, rows: 24 }));
    assert!(encode(&invalid).is_err());
    assert!(decode_client(&rmp_serde::to_vec_named(&invalid).unwrap()).is_err());
    for data in [&b""[..], b"\x00raw", b"\x03{\"type\":\"resize\"}", b"{\"stdin\":[0]}"] {
        assert!(decode_client(data).is_err());
    }
    let mut trailing = encode(&Message::Stdin(b"x")).unwrap();
    trailing.push(0);
    assert!(
        decode_client(&trailing).is_err(),
        "one transport message contains exactly one record"
    );
    for suffix in [&b"\x91"[..], b"\xcc", b"\xd9", b"\x81\xa1k"] {
        let mut truncated = encode(&Message::Stdin(b"x")).unwrap();
        truncated.extend_from_slice(suffix);
        assert!(
            decode_client(&truncated).is_err(),
            "truncated trailing record: {suffix:?}"
        );
    }
    assert!(
        decode_client(b"\x81\xa5stdin\x93\x00\x01\x02").is_err(),
        "bytes must be binary, not integer arrays"
    );
    assert!(
        decode_client(b"\x81\xa5stdin\xa3raw").is_err(),
        "bytes must not be strings"
    );
    assert!(
        decode_client(b"\x81\xa5stdin\xa1\xff").is_err(),
        "invalid UTF-8 strings are not binary either"
    );
    assert!(decode_client(&vec![0; MAX_FRAME_BYTES + 1]).is_err());
    assert!(encode(&Message::Stdin(&vec![0; MAX_DATA_BYTES + 1])).is_err());
    let largest = vec![0; MAX_DATA_BYTES];
    let encoded = encode(&Message::Stdin(&largest)).unwrap();
    assert_eq!(decode_client(&encoded).unwrap(), Message::Stdin(&largest));
}

#[test]
fn byte_stream_framing_is_bounded_big_endian_messagepack() {
    let message = Message::Stdin(b"bytes\0\xff");
    let frame = framed(&message).unwrap();
    assert_eq!(
        u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize,
        frame.len() - 4
    );
    let payload = read_payload(&mut &frame[..]).unwrap();
    assert_eq!(decode_client(&payload).unwrap(), message);
    assert!(read_payload(&mut &frame[..frame.len() - 1]).is_err());
    assert!(read_payload(&mut &(MAX_FRAME_BYTES as u32 + 1).to_be_bytes()[..]).is_err());
}

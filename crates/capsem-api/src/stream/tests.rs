use super::*;

#[test]
fn client_frames_decode_stdin_and_typed_control_only() {
    assert_eq!(
        decode_client_frame(&[0, b'l', b's']).unwrap(),
        ClientFrame::Stdin(b"ls")
    );
    let start = encode_control(&StreamControl::Start {
        kind: StreamKind::Exec,
        command: Some("echo hi".into()),
    });
    assert_eq!(start[0], StreamChannel::Control as u8);
    assert_eq!(
        decode_client_frame(&start).unwrap(),
        ClientFrame::Control(StreamControl::Start {
            kind: StreamKind::Exec,
            command: Some("echo hi".into())
        })
    );
    let resize = encode_control(&StreamControl::Resize { cols: 120, rows: 40 });
    assert_eq!(
        decode_client_frame(&resize).unwrap(),
        ClientFrame::Control(StreamControl::Resize { cols: 120, rows: 40 })
    );
    assert_eq!(
        decode_client_frame(&encode_control(&StreamControl::CloseStdin)).unwrap(),
        ClientFrame::Control(StreamControl::CloseStdin)
    );
}

#[test]
fn server_frames_decode_output_and_status_only() {
    assert_eq!(
        decode_server_frame(&encode_data(StreamChannel::Stdout, b"out")).unwrap(),
        ServerFrame::Stdout(b"out")
    );
    assert_eq!(
        decode_server_frame(&encode_data(StreamChannel::Stderr, b"err")).unwrap(),
        ServerFrame::Stderr(b"err")
    );
    let exit = encode_status(&StreamStatus::Exit {
        code: 3,
        truncated: false,
    });
    assert_eq!(
        decode_server_frame(&exit).unwrap(),
        ServerFrame::Status(StreamStatus::Exit {
            code: 3,
            truncated: false
        })
    );
}

#[test]
fn each_side_refuses_the_other_sides_channels() {
    for frame in [
        encode_data(StreamChannel::Stdout, b"x"),
        encode_status(&StreamStatus::Started),
    ] {
        assert_eq!(
            decode_client_frame(&frame),
            Err(StreamFrameError::WrongDirection(frame[0]))
        );
    }
    for frame in [vec![0, b'x'], encode_control(&StreamControl::CloseStdin)] {
        assert_eq!(
            decode_server_frame(&frame),
            Err(StreamFrameError::WrongDirection(frame[0]))
        );
    }
}

#[test]
fn malformed_frames_are_refused_not_guessed() {
    assert_eq!(decode_client_frame(&[]), Err(StreamFrameError::Empty));
    assert_eq!(
        decode_client_frame(&[9, 1, 2]),
        Err(StreamFrameError::UnknownChannel(9))
    );
    assert!(matches!(
        decode_client_frame(&[3, b'{']),
        Err(StreamFrameError::Control(_))
    ));
    assert!(matches!(
        decode_client_frame(&[3, b'{', b'"', b't', b'y', b'p', b'e', b'"', b':', b'"', b'x', b'"', b'}']),
        Err(StreamFrameError::Control(_))
    ));
    let oversized = vec![0u8; MAX_STREAM_FRAME_BYTES + 1];
    assert_eq!(
        decode_client_frame(&oversized),
        Err(StreamFrameError::TooLarge(oversized.len()))
    );
}

#[test]
fn control_messages_that_cannot_be_honoured_are_refused() {
    for invalid in [
        StreamControl::Resize { cols: 0, rows: 40 },
        StreamControl::Resize { cols: 80, rows: 0 },
        StreamControl::Start {
            kind: StreamKind::Exec,
            command: None,
        },
        StreamControl::Start {
            kind: StreamKind::Exec,
            command: Some(String::new()),
        },
        StreamControl::Start {
            kind: StreamKind::Terminal,
            command: Some("ls".into()),
        },
        StreamControl::Start {
            kind: StreamKind::Container,
            command: Some("ls".into()),
        },
    ] {
        let frame = encode_control(&invalid);
        assert!(
            matches!(decode_client_frame(&frame), Err(StreamFrameError::Control(_))),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn control_wire_shape_is_tagged_snake_case_json() {
    let frame = encode_control(&StreamControl::Start {
        kind: StreamKind::Container,
        command: None,
    });
    assert_eq!(&frame[1..], br#"{"type":"start","kind":"container"}"#);
    let frame = encode_status(&StreamStatus::Error {
        message: "no VM".into(),
    });
    assert_eq!(&frame[1..], br#"{"type":"error","message":"no VM"}"#);
    assert_eq!(STREAM_SUBPROTOCOL, "capsem.stream.v1");
}

/// Finding 32: the web app carries its own copy of this codec. The checked-in
/// fixture is what the web test holds that copy to, byte for byte, so it must
/// be exactly what this module encodes and decodes.
#[test]
fn checked_in_golden_frames_match_the_rust_codec() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/specification/stream-v1.json");
    let checked_in: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default()).unwrap_or(serde_json::Value::Null);
    assert_eq!(
        checked_in,
        golden_frames(),
        "Regenerate sdk/specification/stream-v1.json with the capsem-api export_stream_fixture example"
    );
}

/// Every server-bound expectation in the fixture is the Rust decoder's own
/// answer, including the frames a client must refuse.
#[test]
fn golden_frames_cover_every_channel_and_each_refusal() {
    let golden = golden_frames();
    let kinds: Vec<&str> = golden["server"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["expect"]["kind"].as_str().unwrap())
        .collect();
    for kind in ["output", "status", "invalid"] {
        assert!(kinds.contains(&kind), "no {kind} case: {kinds:?}");
    }
    let controls: Vec<&str> = golden["client"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|case| case["control"]["type"].as_str())
        .collect();
    for control in ["start", "resize", "close_stdin"] {
        assert!(controls.contains(&control), "no {control} control: {controls:?}");
    }
}

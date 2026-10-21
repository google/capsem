use super::*;

#[test]
fn terminal_data_uses_the_shared_named_messagepack_binary_contract() {
    let bytes = [0, 255, 0x1b];
    let mut expected = b"\x81\xa5stdin\xc4\x03".to_vec();
    expected.extend_from_slice(&bytes);
    let encoded = encode_data(StreamChannel::Stdin, &bytes).unwrap();
    assert_eq!(encoded, expected);
    assert_eq!(decode_client_frame(&encoded).unwrap(), ClientFrame::Stdin(&bytes));
}

#[test]
fn typed_controls_and_statuses_round_trip_without_parallel_api_types() {
    for control in [
        StreamControl::Start {
            kind: StreamKind::Terminal,
            command: None,
        },
        StreamControl::Start {
            kind: StreamKind::Exec,
            command: Some("echo hi".into()),
        },
        StreamControl::Resize { cols: 120, rows: 40 },
        StreamControl::Focus { cols: 120, rows: 40 },
        StreamControl::CloseStdin,
    ] {
        let encoded = encode_control(&control).unwrap();
        assert_eq!(decode_client_frame(&encoded).unwrap(), ClientFrame::Control(control));
        assert!(decode_server_frame(&encoded).is_err());
    }
    for status in [
        StreamStatus::Started,
        StreamStatus::Exit {
            code: 7,
            truncated: false,
        },
        StreamStatus::Error {
            message: "closed".into(),
        },
    ] {
        let encoded = encode_status(&status).unwrap();
        assert_eq!(decode_server_frame(&encoded).unwrap(), ServerFrame::Status(status));
        assert!(decode_client_frame(&encoded).is_err());
    }
    for (channel, expected) in [
        (StreamChannel::Stdout, ServerFrame::Stdout(&b"out"[..])),
        (StreamChannel::Stderr, ServerFrame::Stderr(&b"out"[..])),
    ] {
        let encoded = encode_data(channel, b"out").unwrap();
        assert_eq!(decode_server_frame(&encoded).unwrap(), expected);
        assert!(decode_client_frame(&encoded).is_err());
    }
    assert!(decode_server_frame(&encode_data(StreamChannel::Stdin, b"input").unwrap()).is_err());
}

#[test]
fn invalid_messages_and_legacy_protocols_are_refused() {
    for control in [
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
        assert!(encode_control(&control).is_err());
        let invalid = rmp_serde::to_vec_named(&Message::Control(Cow::Owned(control))).unwrap();
        assert!(decode_client_frame(&invalid).is_err());
    }
    for bytes in [&b""[..], b"\x00ls", b"\x03{\"type\":\"close_stdin\"}"] {
        assert!(decode_client_frame(bytes).is_err());
    }
    assert!(decode_client_frame(&vec![0; MAX_STREAM_FRAME_BYTES + 1]).is_err());
    assert!(encode_data(StreamChannel::Stdin, &vec![0; MAX_STREAM_DATA_BYTES + 1]).is_err());
    let largest = vec![0; MAX_STREAM_DATA_BYTES];
    let encoded = encode_data(StreamChannel::Stdout, &largest).unwrap();
    assert_eq!(decode_server_frame(&encoded).unwrap(), ServerFrame::Stdout(&largest));
    assert_eq!(STREAM_SUBPROTOCOL, "capsem.stream.v3");
}

#[test]
fn checked_in_golden_frames_match_the_rust_codec() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/specification/stream-v3.json");
    let actual: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(actual, golden_frames());
}

#[test]
fn golden_frames_cover_outputs_metadata_and_refusals() {
    let golden = golden_frames();
    let kinds: Vec<_> = golden["server"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["expect"]["kind"].as_str().unwrap())
        .collect();
    for kind in ["output", "status", "invalid"] {
        assert!(kinds.contains(&kind));
    }
    let controls: Vec<_> = golden["client"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|case| case["control"]["type"].as_str())
        .collect();
    for control in ["start", "resize", "focus", "close_stdin"] {
        assert!(controls.contains(&control));
    }
}

#[test]
fn terminal_state_is_validated_before_renderer_allocation() {
    let fixture = golden_frames();
    let mut state = fixture["server"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|entry| entry["expect"]["status"].get("state").cloned())
        .unwrap();
    for (key, value) in [
        ("cols", serde_json::json!(0)),
        ("rows", serde_json::json!(65535)),
        (
            "title",
            serde_json::json!("x".repeat(capsem_proto::TERMINAL_METADATA_BYTES + 1)),
        ),
        (
            "events",
            serde_json::json!(vec![
                serde_json::json!({"kind":"bell","visual":false});
                capsem_proto::TERMINAL_EVENTS_PER_UPDATE + 1
            ]),
        ),
    ] {
        let old = state[key].clone();
        state[key] = value;
        let bytes = rmp_serde::to_vec_named(&serde_json::json!({"status":{"type":"terminal","state":state}})).unwrap();
        assert!(decode_server_frame(&bytes).is_err(), "invalid {key}");
        state[key] = old;
    }
}

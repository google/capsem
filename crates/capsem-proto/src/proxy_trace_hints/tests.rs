use super::*;

#[test]
fn fixed_trace_hint_round_trips_with_zeroed_slack() {
    let hint = ProxyTraceHint {
        trace_id: "2e934d32-1e3e-4d51-b816-e14cc662833f".into(),
        relative_path: "reports/result.txt".into(),
    };
    let frame = encode_proxy_trace_hint(&hint).unwrap();
    assert_eq!(frame.len(), PROXY_TRACE_HINT_FRAME_SIZE);
    assert_eq!(decode_proxy_trace_hint(&frame).unwrap(), hint);
}

#[test]
fn trace_hint_rejects_oversize_and_reserved_data() {
    for trace_id in [
        "not-a-trace",
        "0000000000000000",
        "2e934d321e3e-4d51-b816-e14cc662833f",
        "2e934d32-1e3e-4d51-b816-e14cc662833G",
    ] {
        assert_eq!(
            encode_proxy_trace_hint(&ProxyTraceHint {
                trace_id: trace_id.into(),
                relative_path: "result.txt".into(),
            }),
            Err(ProxyTraceHintError::InvalidTraceId)
        );
    }
    assert_eq!(
        encode_proxy_trace_hint(&ProxyTraceHint {
            trace_id: "0707070707070707".into(),
            relative_path: "x".repeat(MAX_TRACE_HINT_PATH_BYTES + 1),
        }),
        Err(ProxyTraceHintError::InvalidPath)
    );
    let mut frame = encode_proxy_trace_hint(&ProxyTraceHint {
        trace_id: "0707070707070707".into(),
        relative_path: "result.txt".into(),
    })
    .unwrap();
    frame[PROXY_TRACE_HINT_FRAME_SIZE - 1] = 1;
    assert_eq!(decode_proxy_trace_hint(&frame), Err(ProxyTraceHintError::ReservedBytes));
}

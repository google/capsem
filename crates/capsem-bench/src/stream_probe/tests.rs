use super::*;

#[test]
fn marker_requires_the_whole_value_across_frames_and_overlapping_prefixes() {
    let mut marker = Marker::new(b"aabaab");
    assert!(!marker.observe(b"ignored-aabaaa"));
    assert!(marker.observe(b"baab"));
    assert!(!marker.observe(b"wrong"));
    assert!(marker.observe(b"aabaab"));
}

#[test]
fn exec_transfer_refuses_wrong_bytes_short_output_errors_and_truncation() {
    assert!(ExecTransfer::default().observe(StreamEvent::Output(vec![1])).is_err());
    assert!(ExecTransfer::default()
        .observe(StreamEvent::ErrorOutput(vec![]))
        .is_err());
    assert!(ExecTransfer::default()
        .observe(StreamEvent::Exit {
            code: 0,
            truncated: false
        })
        .is_err());
    let mut transfer = ExecTransfer {
        received: EXEC_BYTES - 1,
    };
    assert!(!transfer.observe(StreamEvent::Output(vec![0])).unwrap());
    assert!(transfer
        .observe(StreamEvent::Exit {
            code: 1,
            truncated: false
        })
        .is_err());
    assert!(transfer
        .observe(StreamEvent::Exit {
            code: 0,
            truncated: true
        })
        .is_err());
    assert!(transfer
        .observe(StreamEvent::Exit {
            code: 0,
            truncated: false
        })
        .unwrap());
    assert!(transfer.observe(StreamEvent::Output(vec![0])).is_err());
}

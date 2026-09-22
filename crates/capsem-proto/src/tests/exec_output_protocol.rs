use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "t", content = "d", rename_all = "lowercase")]
enum LegacyGuestToHost {
    ExecStarted { id: u64 },
}

#[test]
fn roundtrip_exec_started() {
    let msg = GuestToHost::ExecStarted {
        id: 42,
        output_protocol: ExecOutputProtocol::FramedLanes,
    };
    let frame = encode_guest_msg(&msg).unwrap();
    let decoded = decode_guest_msg(&frame[4..]).unwrap();
    match decoded {
        GuestToHost::ExecStarted { id, output_protocol } => {
            assert_eq!(id, 42);
            assert_eq!(output_protocol, ExecOutputProtocol::FramedLanes);
        }
        other => panic!("expected ExecStarted, got {other:?}"),
    }
}

#[test]
fn legacy_exec_started_defaults_to_raw_merged_output() {
    let payload = rmp_serde::to_vec_named(&LegacyGuestToHost::ExecStarted { id: 7 }).unwrap();
    let decoded = decode_guest_msg(&payload).unwrap();
    match decoded {
        GuestToHost::ExecStarted { id, output_protocol } => {
            assert_eq!(id, 7);
            assert_eq!(output_protocol, ExecOutputProtocol::RawMerged);
        }
        other => panic!("expected ExecStarted, got {other:?}"),
    }

    let current = GuestToHost::ExecStarted {
        id: 8,
        output_protocol: ExecOutputProtocol::FramedLanes,
    };
    let payload = rmp_serde::to_vec_named(&current).unwrap();
    let legacy: LegacyGuestToHost = rmp_serde::from_slice(&payload).unwrap();
    assert!(matches!(legacy, LegacyGuestToHost::ExecStarted { id: 8 }));
}

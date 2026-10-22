use super::*;

#[test]
fn roundtrip_prepare_snapshot() {
    let frame = encode_host_msg(&HostToGuest::PrepareSnapshot).unwrap();
    assert!(matches!(
        decode_host_msg(&frame[4..]).unwrap(),
        HostToGuest::PrepareSnapshot
    ));
}

#[test]
fn roundtrip_unfreeze() {
    let frame = encode_host_msg(&HostToGuest::Unfreeze).unwrap();
    assert!(matches!(decode_host_msg(&frame[4..]).unwrap(), HostToGuest::Unfreeze));
}

#[test]
fn roundtrip_snapshot_ready() {
    let frame = encode_guest_msg(&GuestToHost::SnapshotReady).unwrap();
    assert!(matches!(
        decode_guest_msg(&frame[4..]).unwrap(),
        GuestToHost::SnapshotReady
    ));
}

#[test]
fn filesystem_snapshot_transitions_preserve_correlation_phase_and_errors() {
    for frozen in [true, false] {
        let request = HostToGuest::SnapshotTransition { id: u64::MAX, frozen };
        let frame = encode_host_msg(&request).unwrap();
        let decoded = decode_host_msg(&frame[4..]).unwrap();
        assert!(
            matches!(decoded, HostToGuest::SnapshotTransition { id: u64::MAX, frozen: actual } if actual == frozen)
        );
        for error in [None, Some("filesystem remains frozen".to_owned())] {
            let frame = encode_guest_msg(&GuestToHost::SnapshotTransitioned {
                id: u64::MAX,
                frozen,
                error: error.clone(),
            })
            .unwrap();
            let decoded = decode_guest_msg(&frame[4..]).unwrap();
            match decoded {
                GuestToHost::SnapshotTransitioned {
                    id,
                    frozen: actual,
                    error: actual_error,
                } => {
                    assert_eq!(id, u64::MAX);
                    assert_eq!(actual, frozen);
                    assert_eq!(actual_error, error);
                }
                other => panic!("unexpected snapshot response: {other:?}"),
            }
        }
    }
}

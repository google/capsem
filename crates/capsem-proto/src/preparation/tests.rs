use super::*;

#[test]
fn preparation_status_roundtrips_and_refuses_extra_data() {
    for event in [
        PreparationStatus::Preparing,
        PreparationStatus::Entrypoint,
        PreparationStatus::Failed {
            failure: PreparationFailure::Exit { code: 17 },
        },
        PreparationStatus::Failed {
            failure: PreparationFailure::TimedOut,
        },
    ] {
        let bytes = encode(&event).unwrap();
        let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(length, bytes.len() - 4);
        assert!(length <= MAX_STATUS_BYTES as usize);
        assert_eq!(decode(&bytes[4..]).unwrap(), event);
        let mut trailing = bytes[4..].to_vec();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
    }
    // A fixed named record also serves the Python controller's codec contract.
    assert_eq!(
        &encode(&PreparationStatus::Preparing).unwrap()[4..],
        b"\x81\xa4type\xa9preparing"
    );
    for failure in [
        PreparationFailure::Exit { code: 0 },
        PreparationFailure::Exit { code: 256 },
        PreparationFailure::Signal { signal: 0 },
        PreparationFailure::Signal { signal: 65 },
    ] {
        let status = PreparationStatus::Failed { failure };
        assert!(encode(&status).is_err());
        let bytes = crate::framing::encode_record(&status, MAX_STATUS_BYTES as usize).unwrap();
        assert!(decode(&bytes).is_err());
    }
    assert!(decode(&[]).is_err());
    assert!(decode(&vec![0; MAX_STATUS_BYTES as usize + 1]).is_err());
}

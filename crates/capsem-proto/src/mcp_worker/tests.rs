use super::*;

#[test]
fn builtin_http_admission_receipts_are_explicit_and_strict() {
    for admission in [
        McpHttpAdmission::Allowed {
            rule_id: Some("exact-rule".into()),
            reason: None,
        },
        McpHttpAdmission::Denied {
            reason: "blocked".into(),
        },
    ] {
        assert_eq!(
            serde_json::from_slice::<McpHttpAdmission>(&serde_json::to_vec(&admission).unwrap()).unwrap(),
            admission
        );
    }
    for hostile in [
        r#"{"status":"unknown"}"#,
        r#"{"status":"denied"}"#,
        r#"{"status":"allowed","credential":"forged"}"#,
        r#"{"status":"allowed","reason":false}"#,
        r#"{"allowed":true}"#,
    ] {
        assert!(
            serde_json::from_str::<McpHttpAdmission>(hostile).is_err(),
            "accepted {hostile}"
        );
    }
}

#[test]
fn worker_roles_and_ready_direction_have_exact_descriptor_shapes() {
    for role in [McpHttpRole::Aggregator, McpHttpRole::Builtin] {
        let bootstrap = McpWorkerBootstrap {
            role,
            generation: [9; 16],
        };
        for record in [McpWorkerRecord::Bootstrap(bootstrap), McpWorkerRecord::Ready(bootstrap)] {
            assert_eq!(
                decode_worker_record(&encode_worker_record(record).unwrap()).unwrap(),
                record
            );
        }
        assert_eq!(
            McpWorkerRecord::Bootstrap(bootstrap).expected_descriptor_count(),
            if role == McpHttpRole::Aggregator { 2 } else { 1 }
        );
        assert_eq!(McpWorkerRecord::Ready(bootstrap).expected_descriptor_count(), 0);
    }
}

#[test]
fn bootstrap_rejects_unknown_role_zero_generation_and_reserved_bytes() {
    let record = McpWorkerRecord::Bootstrap(McpWorkerBootstrap {
        role: McpHttpRole::Builtin,
        generation: [1; 16],
    });
    let valid = encode_worker_record(record).unwrap();
    for index in [0, 1, 2, 3, 4, 5, 6, 7] {
        let mut mutated = valid;
        mutated[index] = 255;
        assert!(decode_worker_record(&mutated).is_err(), "accepted mutated byte {index}");
    }
    let mut zero = valid;
    zero[8..].fill(0);
    assert!(decode_worker_record(&zero).is_err());
    assert!(encode_worker_record(McpWorkerRecord::Ready(McpWorkerBootstrap {
        role: McpHttpRole::Builtin,
        generation: [0; 16]
    }))
    .is_err());
}

#[test]
fn builtin_spawn_has_no_arbitrary_command_or_path_and_fences_reply_ids() {
    for record in [
        McpSpawnRecord::Builtin {
            request_id: 7,
            peer_index: 0,
        },
        McpSpawnRecord::Granted { request_id: 7 },
        McpSpawnRecord::Denied { request_id: 7 },
    ] {
        let encoded = encode_spawn_record(record).unwrap();
        assert_eq!(decode_spawn_record(&encoded).unwrap(), record);
        assert_eq!(
            record.expected_descriptor_count(),
            usize::from(matches!(record, McpSpawnRecord::Granted { .. }))
        );
    }
    assert!(encode_spawn_record(McpSpawnRecord::Builtin {
        request_id: 0,
        peer_index: 0
    })
    .is_err());
    let granted = encode_spawn_record(McpSpawnRecord::Granted { request_id: 9 }).unwrap();
    for index in [0, 1, 2, 3, 12, 13, 14, 15] {
        let mut mutated = granted;
        mutated[index] = 255;
        assert!(decode_spawn_record(&mutated).is_err());
    }
    let mut zero = granted;
    zero[4..12].fill(0);
    assert!(decode_spawn_record(&zero).is_err());
}

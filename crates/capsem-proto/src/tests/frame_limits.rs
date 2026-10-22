use super::*;

#[test]
fn all_host_variants_fit() {
    let messages = vec![
        HostToGuest::BootConfig {
            epoch_secs: u64::MAX,
            traceparent: String::new(),
        },
        HostToGuest::SetEnv {
            key: "K".into(),
            value: "V".into(),
        },
        HostToGuest::BootConfigDone,
        HostToGuest::Resize {
            cols: u16::MAX,
            rows: u16::MAX,
        },
        HostToGuest::Exec {
            id: u64::MAX,
            command: "echo hello".into(),
        },
        HostToGuest::Ping { epoch_secs: 0 },
        HostToGuest::FileWrite {
            id: u64::MAX,
            path: "/test".into(),
            data: vec![0; 10],
            mode: 0o644,
        },
        HostToGuest::FileRead {
            id: 1,
            path: "/test".into(),
        },
        HostToGuest::FileDelete {
            id: u64::MAX,
            path: "/test".into(),
        },
        HostToGuest::Shutdown,
        HostToGuest::SnapshotTransition { id: 13, frozen: true },
    ];
    for msg in messages {
        let frame = encode_host_msg(&msg).unwrap();
        let payload_len = frame.len() - 4;
        assert!(
            payload_len <= MAX_FRAME_SIZE as usize,
            "{msg:?} payload is {payload_len} bytes, exceeds max {MAX_FRAME_SIZE}"
        );
    }
}

#[test]
fn all_guest_variants_fit() {
    let messages = vec![
        GuestToHost::Ready {
            version: "99.99.99".into(),
        },
        GuestToHost::BootReady,
        GuestToHost::BootTiming {
            stages: vec![
                BootStage {
                    name: "squashfs".into(),
                    duration_ms: 50,
                },
                BootStage {
                    name: "network".into(),
                    duration_ms: 120,
                },
            ],
        },
        GuestToHost::ExecDone {
            id: u64::MAX,
            exit_code: i32::MIN,
        },
        GuestToHost::Pong,
        GuestToHost::FileCreated {
            path: "/test".into(),
            size: u64::MAX,
        },
        GuestToHost::FileModified {
            path: "/test".into(),
            size: u64::MAX,
        },
        GuestToHost::FileDeleted { path: "/test".into() },
        GuestToHost::FileContent {
            id: 1,
            path: "/test".into(),
            data: vec![0; 10],
        },
        GuestToHost::ShutdownRequest,
        GuestToHost::SuspendRequest,
        GuestToHost::SnapshotTransitioned {
            id: 13,
            frozen: true,
            error: None,
        },
        GuestToHost::ShutdownComplete,
    ];
    for msg in messages {
        let frame = encode_guest_msg(&msg).unwrap();
        let payload_len = frame.len() - 4;
        assert!(
            payload_len <= MAX_FRAME_SIZE as usize,
            "{msg:?} payload is {payload_len} bytes, exceeds max {MAX_FRAME_SIZE}"
        );
    }
}

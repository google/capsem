use super::*;

// ── Ack-eligible message sets ──────────────────────────────────────
//
// ackable_id and ackable_response_id decide which messages join the retry /
// replay path: the sender keeps them pending and replays them on every fresh
// control connection until an AckReply arrives. Getting the set wrong is
// silent both ways -- a missing variant is a message that can be lost across
// a reconnect, an extra one is a message replayed forever. Neither had a test.

#[test]
fn every_host_to_guest_side_effect_is_ack_eligible() {
    let cases: Vec<(HostToGuest, u64)> = vec![
        (
            HostToGuest::Exec {
                id: 1,
                command: "ls".into(),
            },
            1,
        ),
        (
            HostToGuest::FileWrite {
                id: 2,
                path: "/w/a".into(),
                data: vec![1, 2, 3],
                mode: 0o644,
            },
            2,
        ),
        (
            HostToGuest::FileRead {
                id: 3,
                path: "/w/b".into(),
            },
            3,
        ),
        (
            HostToGuest::FileDelete {
                id: 4,
                path: "/w/c".into(),
            },
            4,
        ),
    ];

    for (msg, want) in cases {
        assert_eq!(
            ackable_id(&msg),
            Some(want),
            "{msg:?} performs guest-side work and must survive a reconnect"
        );
    }
}

#[test]
fn control_chatter_is_not_ack_eligible() {
    // Replaying these would be noise at best; Shutdown replayed after a
    // reconnect would be actively wrong.
    for msg in [
        HostToGuest::Ping { epoch_secs: 0 },
        HostToGuest::Shutdown,
        HostToGuest::AckReply { id: 9 },
    ] {
        assert_eq!(ackable_id(&msg), None, "{msg:?} must not be replayed");
    }
}

#[test]
fn every_guest_to_host_completion_is_ack_eligible() {
    let cases: Vec<(GuestToHost, u64)> = vec![
        (GuestToHost::ExecDone { id: 10, exit_code: 0 }, 10),
        (GuestToHost::FileOpDone { id: 11 }, 11),
        (
            GuestToHost::FileContent {
                id: 12,
                path: "/w/b".into(),
                data: vec![0xde, 0xad],
            },
            12,
        ),
        (
            GuestToHost::Error {
                id: 13,
                message: "denied".into(),
            },
            13,
        ),
    ];

    for (msg, want) in cases {
        assert_eq!(
            ackable_response_id(&msg),
            Some(want),
            "{msg:?} is a completion the host must not lose"
        );
    }
}

#[test]
fn guest_liveness_messages_are_not_ack_eligible() {
    for msg in [GuestToHost::Pong, GuestToHost::Ready { version: "1.0".into() }] {
        assert_eq!(
            ackable_response_id(&msg),
            None,
            "{msg:?} carries no correlation id to ack"
        );
    }
}

#[test]
fn only_periodic_pong_is_expected_post_handshake_liveness() {
    assert!(is_guest_liveness_message(&GuestToHost::Pong));
    assert!(!is_guest_liveness_message(&GuestToHost::Ready {
        version: "1.0".into(),
    }));
    assert!(!is_guest_liveness_message(&GuestToHost::Error {
        id: 9,
        message: "broken".into(),
    }));
}

#[test]
fn the_two_directions_agree_on_which_ids_they_carry() {
    // A request that is ack-eligible must have a completion that is too,
    // otherwise one half of the pair survives a reconnect and the other does
    // not, and the caller waits forever on a reply that was dropped.
    let request = HostToGuest::FileRead {
        id: 77,
        path: "/w/x".into(),
    };
    let completion = GuestToHost::FileContent {
        id: 77,
        path: "/w/x".into(),
        data: vec![],
    };

    assert_eq!(ackable_id(&request), ackable_response_id(&completion));
}

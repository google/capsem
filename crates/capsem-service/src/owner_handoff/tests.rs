use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_test_state_owned};

#[tokio::test]
async fn handoffs_use_the_trusted_identity_with_short_and_hashed_paths() {
    for long in [false, true] {
        let mut owned = make_test_state_owned();
        if long {
            owned.run_dir = owned.run_dir.join("long-runtime-root-".repeat(8));
        }
        let state = Arc::new(owned);
        insert_fake_instance_with_session_dir(&state, "session", 1, state.run_dir.join("sessions/session"));
        let expected = capsem_foundation::uds::private_handoff_socket_path(&state.run_dir, "session").unwrap();
        let handoff = OwnerHandoff::acquire(&state, "session").await.unwrap();
        assert_eq!(handoff.validate(&state, expected.to_str().unwrap()).unwrap(), expected);
        if long {
            assert!(
                !expected.starts_with(&state.run_dir),
                "exercise the stable hashed fallback"
            );
        } else {
            assert!(expected.starts_with(&state.run_dir));
        }
    }
}

#[tokio::test]
async fn handoff_grants_are_revoked_when_the_owner_is_removed_or_replaced() {
    for replace in [false, true] {
        let state = crate::tests::make_test_state();
        insert_fake_instance_with_session_dir(&state, "session", 1, state.run_dir.join("sessions/session"));
        let expected = capsem_foundation::uds::private_handoff_socket_path(&state.run_dir, "session").unwrap();
        let handoff = OwnerHandoff::acquire(&state, "session").await.unwrap();
        state.instances.lock().unwrap().remove("session");
        if replace {
            insert_fake_instance_with_session_dir(&state, "session", 1, state.run_dir.join("sessions/session"));
        }
        let error = handoff.validate(&state, expected.to_str().unwrap()).unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_GATEWAY);
        assert_eq!(error.1, "VM owner changed during handoff admission");
    }
}

#[tokio::test]
async fn stopped_sessions_have_no_owner_handoff_grant() {
    let state = crate::tests::make_test_state();
    let error = OwnerHandoff::acquire(&state, "stopped").await.err().unwrap();
    assert_eq!(error.0, StatusCode::NOT_FOUND);
}

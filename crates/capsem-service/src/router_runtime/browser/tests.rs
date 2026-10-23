use super::*;
use crate::tests::{make_test_state, test_instance};

#[test]
fn delayed_ack_cannot_clear_replacement_owner_with_reused_request_id() {
    let state = make_test_state();
    let old_generation = capsem_crypto::uuid_v4();
    let mut instance = test_instance();
    let id = instance.id.clone();
    let generation = instance.generation;
    instance.observe_browser(capsem_api::BrowserRequest {
        owner_generation: None,
        id: 1,
        url: "https://example.test/".into(),
    });
    state.instances.lock().unwrap().insert(id.clone(), instance);
    assert_eq!(
        owner_path(&state, &id, old_generation).unwrap_err().status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        clear(&state, &id, old_generation, 1).unwrap_err().status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        state.instances.lock().unwrap()[&id]
            .browser_request
            .as_ref()
            .unwrap()
            .id,
        1
    );
    clear(&state, &id, generation, 1).unwrap();
    assert!(state.instances.lock().unwrap()[&id].browser_request.is_none());
}

#[test]
fn missing_browser_owner_is_not_found() {
    assert_eq!(
        owner_path(&make_test_state(), "absent", capsem_crypto::uuid_v4())
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
}

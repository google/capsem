use super::*;
use capsem_api::{AgentAccessScope, AgentOperation, AgentSessionScope};
use capsem_core::{agent_access::Reservation, managed_sessions::Capability};

#[test]
fn a_read_admission_cannot_be_reused_for_execute_even_when_the_grant_permits_both() {
    let state = crate::tests::make_test_state();
    let id = capsem_crypto::uuid_v4().to_string();
    crate::tests::insert_fake_instance_with_session_dir(&state, &id, std::process::id(), state.run_dir.join(&id));
    let generation = state.instances.lock().unwrap()[&id].generation;
    let access = capsem_crypto::uuid_v4();
    let scope = AgentAccessScope::new(
        vec![AgentOperation::Inspect, AgentOperation::Execute],
        vec![],
        vec![AgentSessionScope::new(id.clone(), generation.to_string()).unwrap()],
    )
    .unwrap();
    let Reservation::New(admission) = state
        .caller_authority
        .agents
        .reserve(
            access,
            &Capability::from_bytes([0x11; 32]),
            scope,
            std::time::Duration::from_secs(60),
            || LeaseClock::now().ok(),
        )
        .unwrap()
    else {
        panic!("new admission");
    };
    admission.commit(|| LeaseClock::now().ok()).unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {access}.{}", "11".repeat(32)).parse().unwrap(),
    );
    let mut actor = state.caller_authority.authenticate_gateway(&headers).unwrap();
    assert!(actor.bind_session(&state, &id, AgentOperation::Inspect));
    let owner = actor.session_owner(&state, &id, AgentOperation::Inspect).unwrap();
    assert!(actor.check_session(&state, &owner, AgentOperation::Execute).is_err());
    assert!(actor.session_owner(&state, &id, AgentOperation::Execute).is_err());
    assert!(actor.bind_session(&state, &id, AgentOperation::Execute));
    let owner = actor.session_owner(&state, &id, AgentOperation::Execute).unwrap();
    assert!(actor.check_session(&state, &owner, AgentOperation::Execute).is_ok());
    assert!(actor.check_session(&state, &owner, AgentOperation::Inspect).is_err());
    assert!(!actor.bind_session(&state, &capsem_crypto::uuid_v4().to_string(), AgentOperation::Inspect));
    assert!(
        actor.session_owner(&state, &id, AgentOperation::Execute).is_err(),
        "failed rebinding must discard prior admission"
    );
}

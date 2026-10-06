use super::*;
use crate::tests::{make_test_state, test_instance};

#[test]
fn teardown_claim_is_bound_to_generation_even_when_id_and_pid_are_reused() {
    let state = make_test_state();
    let original = test_instance();
    let generation = original.generation;
    let pid = original.pid;
    state.instances.lock().unwrap().insert("vm".into(), original);
    let mut replacement = test_instance();
    replacement.pid = pid;
    let replacement_generation = replacement.generation;
    assert_ne!(generation, replacement_generation);
    state.instances.lock().unwrap().insert("vm".into(), replacement);
    assert!(!claim_shutdown_instance(&state, "vm", generation));
    assert_eq!(
        state.instances.lock().unwrap().get("vm").unwrap().generation,
        replacement_generation
    );
    assert!(claim_shutdown_instance(&state, "vm", replacement_generation));
    assert!(!claim_shutdown_instance(&state, "vm", replacement_generation));
}

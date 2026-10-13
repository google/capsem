use super::*;

#[tokio::test]
async fn completed_switch_retirement_is_awaited_despite_another_network_failure() {
    let state = crate::tests::make_test_state();
    let good = state.networks.lock().await.create("completed", 1000).await.unwrap();
    let switch = host(&state, good.id).await.unwrap();
    assert!(!switch.is_closed());
    {
        let mut registry = state.networks.lock().await;
        registry.attach(good.id, "completed-vm", 1001).await.unwrap();
        registry.vm_deleted("completed-vm", 1002).await.unwrap();
        drop(registry);
    }
    let future = i64::MAX / 2;
    let bad = state.networks.lock().await.create("pending", future).await.unwrap();
    state
        .networks
        .lock()
        .await
        .attach(bad.id, "pending-vm", future)
        .await
        .unwrap();
    assert!(crate::network_routes::vm_deleted_checked(&state, "pending-vm")
        .await
        .is_err());
    assert!(
        switch.is_closed(),
        "already retired switch must be awaited on the error path"
    );
    assert!(!state.switches.networks.lock().await.contains_key(&good.id));
    assert!(state.networks.lock().await.address_of(bad.id, "pending-vm").is_some());
    state
        .networks
        .lock()
        .await
        .vm_deleted("pending-vm", future + 1)
        .await
        .unwrap();
    crate::network_routes::vm_deleted_checked(&state, "pending-vm")
        .await
        .unwrap();
}

use super::*;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[tokio::test(start_paused = true)]
async fn concurrent_demo_requests_wait_for_body_capacity_instead_of_returning_502() {
    let budget = Arc::new(Semaphore::new(2));
    let first = admit(&budget).await.unwrap();
    let _second = admit(&budget).await.unwrap();
    let mut queued = tokio::spawn({
        let budget = Arc::clone(&budget);
        async move { admit(&budget).await }
    });
    tokio::task::yield_now().await;
    assert!(!queued.is_finished(), "a normal concurrency burst must wait, not fail");
    assert_eq!(budget.available_permits(), 0);
    drop(first);
    let third = (&mut queued).await.unwrap().unwrap();
    drop(queued);
    assert_eq!(budget.available_permits(), 0);
    drop(third);
    assert_eq!(budget.available_permits(), 1);
}

#[tokio::test(start_paused = true)]
async fn stalled_body_admission_is_bounded_and_does_not_bypass_inspection() {
    let budget = Arc::new(Semaphore::new(0));
    let mut queued = tokio::spawn({
        let budget = Arc::clone(&budget);
        async move { admit(&budget).await }
    });
    tokio::task::yield_now().await;
    assert!(!queued.is_finished());
    tokio::time::advance(BODY_DEADLINE).await;
    let error = (&mut queued).await.unwrap().unwrap_err();
    drop(queued);
    assert!(error.to_string().contains("timed out"));
}

#[test]
fn typed_policy_body_admission_keeps_plugin_directions_and_media_types() {
    let options = toml::from_str(
        r#"
[pii_filter.inbound]
mode = "block"
[pii_filter.outbound]
mode = "disable"
[utf8_guard]
mode = "block"
[utf8_guard.options]
inbound = false
outbound = true
"#,
    )
    .unwrap();
    let plugins =
        Arc::new(crate::environment::CompiledEnvironmentPlugins::resolve(&options, &Default::default()).unwrap());
    let snapshot = crate::net::proxy_engine::ProxyPolicySnapshot::new(
        "typed-admission".into(),
        crate::net::policy::NetworkMechanics::new(),
        crate::net::policy_config::SecurityRuleSet::new(Vec::new()),
        plugins,
        crate::net::policy_config::ModelEndpointRegistry::default(),
    );
    assert!(inspection_enabled(&snapshot, true));
    assert!(!inspection_enabled(&snapshot, false));
    let mut headers = http::HeaderMap::new();
    assert!(!utf8_enabled(&snapshot, true, &headers));
    assert!(utf8_enabled(&snapshot, false, &headers));
    headers.insert(http::header::CONTENT_TYPE, "application/octet-stream".parse().unwrap());
    assert!(!utf8_enabled(&snapshot, false, &headers));
    headers.insert(
        http::header::CONTENT_TYPE,
        "application/problem+json; charset=utf-8".parse().unwrap(),
    );
    assert!(utf8_enabled(&snapshot, false, &headers));
}

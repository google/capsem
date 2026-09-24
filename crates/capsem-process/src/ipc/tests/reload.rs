use super::*;

/// A reload that cannot load its active profile keeps the previous policy in
/// force and says so. It used to drop the connection without a reply, which the
/// service could only report as "IPC connection closed".
#[tokio::test]
async fn failed_reload_answers_with_its_error_and_keeps_the_previous_policy() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, _ctrl_rx) = Dispatcher::new(temp.path());
    let rules_before = Arc::clone(&dispatcher.mcp_runtime.security_rules.read().unwrap());
    let (process_stream, service_stream) = tokio::net::UnixStream::pair().unwrap();
    let handler = dispatcher.connection(process_stream);
    let mut service_stream = service_stream.into_std().unwrap();
    let (service_tx, service_rx): (Sender<ServiceToProcess>, Receiver<ProcessToService>) =
        tokio::task::spawn_blocking(move || {
            capsem_foundation::ipc_handshake::negotiate_initiator(&mut service_stream, "capsem-service-test", "")
                .unwrap();
            channel_from_std(service_stream).unwrap()
        })
        .await
        .unwrap();

    std::fs::write(temp.path().join("active_profile.toml"), "id = [not toml").unwrap();
    service_tx.send(ServiceToProcess::ReloadConfig { id: 7 }).await.unwrap();
    match service_rx.recv().await.unwrap() {
        ProcessToService::ConfigReloadResult {
            id: 7,
            active_profile_digest: None,
            error: Some(error),
        } => assert!(error.contains("active_profile.toml"), "{error}"),
        other => panic!("unexpected reload reply: {other:?}"),
    }
    assert!(
        Arc::ptr_eq(&rules_before, &dispatcher.mcp_runtime.security_rules.read().unwrap()),
        "a failed reload must leave the previous rules in force"
    );
    // The connection survives the failure and keeps answering.
    service_tx.send(ServiceToProcess::Ping).await.unwrap();
    assert!(matches!(service_rx.recv().await.unwrap(), ProcessToService::Pong));
    drop(service_tx);
    handler.await.unwrap().unwrap();
}

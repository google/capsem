use super::*;

#[tokio::test]
async fn gateway_runtime_url_rejects_cross_instance_fallbacks() {
    let run_dir = tempfile::tempdir().unwrap();
    assert!(gateway_url_from_runtime(run_dir.path()).await.is_none());

    std::fs::write(run_dir.path().join("gateway.port"), "not-a-port\n").unwrap();
    assert!(gateway_url_from_runtime(run_dir.path()).await.unwrap().is_err());

    std::fs::write(run_dir.path().join("gateway.port"), "0\n").unwrap();
    assert!(gateway_url_from_runtime(run_dir.path()).await.unwrap().is_err());

    std::fs::write(run_dir.path().join("gateway.port"), "49152\n").unwrap();
    assert_eq!(
        gateway_url_from_runtime(run_dir.path()).await.unwrap().unwrap(),
        "http://127.0.0.1:49152"
    );
}

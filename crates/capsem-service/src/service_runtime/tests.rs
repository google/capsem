use super::*;

#[test]
fn gateway_readiness_requires_complete_stamped_runtime_files() {
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("gateway.token");
    let port = dir.path().join("gateway.port");
    let preview = dir.path().join("preview.port");
    for path in [&token, &port, &preview] {
        std::fs::write(path, "").unwrap();
    }
    assert!(!gateway_runtime_ready(&token, &port, &preview));

    std::fs::write(&token, "a".repeat(64)).unwrap();
    std::fs::write(&port, "19222").unwrap();
    std::fs::write(&preview, "19223").unwrap();
    assert!(gateway_runtime_ready(&token, &port, &preview));

    std::fs::write(&token, "short").unwrap();
    assert!(!gateway_runtime_ready(&token, &port, &preview));
    std::fs::write(&token, "a".repeat(64)).unwrap();
    std::fs::write(&preview, "0").unwrap();
    assert!(!gateway_runtime_ready(&token, &port, &preview));
}

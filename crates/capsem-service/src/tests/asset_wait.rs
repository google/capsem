use super::*;

/// Poll `/assets/status` until no reconciliation is running.
pub(super) async fn wait_for_assets(app: &Router) -> serde_json::Value {
    for _ in 0..100 {
        let (status, body) = route_request(app.clone(), axum::http::Method::GET, "/assets/status", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if !body["downloading"].as_bool().unwrap_or(false) {
            return body;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("VM assets did not settle within one second")
}

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn snapshot_pagination_params_preserve_include_changes() {
    let params: SnapshotPaginationParams = serde_json::from_value(serde_json::json!({
        "format": "json",
        "include_changes": true
    }))
    .expect("snapshot pagination params should deserialize");

    let args = to_args(&params);
    assert_eq!(args["format"], "json");
    assert_eq!(args["include_changes"], true);
}

async fn spawn_one_response_http_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local HTTP fixture");
    let addr = listener.local_addr().expect("fixture local addr");
    tokio::spawn(async move {
        let Ok((mut socket, _peer)) = listener.accept().await else {
            return;
        };
        let mut buf = [0_u8; 1024];
        let _ = socket.read(&mut buf).await;
        let response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "content-type: text/plain; charset=utf-8\r\n",
            "x-capsem-fixture: builtin-flush\r\n",
            "content-length: 0\r\n",
            "\r\n"
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    format!("http://{addr}/")
}

#[tokio::test]
async fn http_builtin_flushes_net_event_before_tool_response_returns() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("session.db");
    let db = Arc::new(DbWriter::open(&db_path, 16).expect("open test db"));
    let handler = BuiltinHandler {
        http_client: reqwest::Client::new(),
        db: Arc::clone(&db),
        security_rules: Arc::new(SecurityRuleSet::new(Vec::new())),
        plugin_policy: Arc::new(BTreeMap::new()),
        scheduler: None,
        workspace_dir: None,
    };
    let url = spawn_one_response_http_server().await;

    let text = call_builtin(
        &handler,
        "http_headers",
        serde_json::json!({"url": url, "method": "HEAD"}),
    )
    .await
    .expect("builtin call succeeds");
    assert!(text.contains("Status: 200"), "{text}");

    let rows = db
        .reader()
        .expect("reader")
        .recent_net_events(10)
        .expect("recent net events");
    assert!(
        rows.iter().any(|row| row.domain == "127.0.0.1"
            && row.method.as_deref() == Some("HEAD")
            && row.decision == capsem_logger::Decision::Allowed),
        "net event must be durable before returning tool response: {rows:?}"
    );
}

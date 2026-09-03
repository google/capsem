use super::*;
use tokio::io::AsyncWriteExt;

/// Roundtrip helper: serialize to msgpack and back.
fn msgpack_roundtrip<T: Serialize + for<'de> Deserialize<'de>>(val: &T) -> T {
    let bytes = rmp_serde::to_vec_named(val).unwrap();
    rmp_serde::from_slice(&bytes).unwrap()
}

#[test]
fn request_list_servers_roundtrip() {
    let req = AggregatorRequest {
        id: 1,
        method: AggregatorMethod::ListServers,
    };
    let decoded = msgpack_roundtrip(&req);
    assert_eq!(decoded.id, 1);
    assert!(matches!(decoded.method, AggregatorMethod::ListServers));
}

#[test]
fn request_call_tool_roundtrip() {
    let req = AggregatorRequest {
        id: 42,
        method: AggregatorMethod::CallTool {
            name: "github__search_repos".into(),
            arguments: serde_json::json!({"query": "rust"}),
            timeout_ms: None,
        },
    };
    let decoded = msgpack_roundtrip(&req);
    assert_eq!(decoded.id, 42);
    if let AggregatorMethod::CallTool { name, arguments, .. } = decoded.method {
        assert_eq!(name, "github__search_repos");
        assert_eq!(arguments["query"], "rust");
    } else {
        panic!("expected CallTool");
    }
}

#[test]
fn request_shutdown_roundtrip() {
    let req = AggregatorRequest {
        id: 99,
        method: AggregatorMethod::Shutdown,
    };
    let decoded = msgpack_roundtrip(&req);
    assert!(matches!(decoded.method, AggregatorMethod::Shutdown));
}

#[test]
fn request_refresh_roundtrip() {
    let req = AggregatorRequest {
        id: 10,
        method: AggregatorMethod::Refresh {
            servers: vec![McpServerDef {
                name: "test".into(),
                url: "https://mcp.example.com".into(),
                command: None,
                args: vec![],
                env: Default::default(),
                headers: Default::default(),
                auth: None,
                enabled: true,
                source: "manual".into(),
                pool_size: None,
                pool_safe_tools: Vec::new(),
            }],
        },
    };
    let decoded = msgpack_roundtrip(&req);
    if let AggregatorMethod::Refresh { servers } = decoded.method {
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "test");
    } else {
        panic!("expected Refresh");
    }
}

#[test]
fn response_servers_roundtrip() {
    let resp = AggregatorResponse {
        id: 1,
        body: AggregatorResult::Servers {
            servers: vec![AggregatorServerStatus {
                name: "github".into(),
                url: "https://mcp.github.com".into(),
                enabled: true,
                source: "claude".into(),
                is_stdio: false,
                connected: true,
                tool_count: 5,
                resource_count: 0,
                prompt_count: 0,
            }],
        },
    };
    let decoded = msgpack_roundtrip(&resp);
    assert_eq!(decoded.id, 1);
    if let AggregatorResult::Servers { servers } = decoded.body {
        assert_eq!(servers[0].name, "github");
        assert!(servers[0].connected);
    } else {
        panic!("expected Servers");
    }
}

#[test]
fn response_error_roundtrip() {
    let resp = AggregatorResponse {
        id: 2,
        body: AggregatorResult::Error {
            error: "server not found".into(),
        },
    };
    let decoded = msgpack_roundtrip(&resp);
    if let AggregatorResult::Error { error } = decoded.body {
        assert_eq!(error, "server not found");
    } else {
        panic!("expected Error");
    }
}

#[test]
fn response_ok_roundtrip() {
    let resp = AggregatorResponse {
        id: 3,
        body: AggregatorResult::Ok { ok: true },
    };
    let decoded = msgpack_roundtrip(&resp);
    if let AggregatorResult::Ok { ok } = decoded.body {
        assert!(ok);
    } else {
        panic!("expected Ok");
    }
}

#[test]
fn aggregator_subprocess_remains_session_db_free() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("capsem-core should live under crates/");
    let files = [
        repo_root.join("crates/capsem-mcp-aggregator/Cargo.toml"),
        repo_root.join("crates/capsem-mcp-aggregator/src/main.rs"),
    ];
    let forbidden = [
        "capsem-logger",
        "capsem_logger",
        "rusqlite",
        "DbWriter",
        "DbReader",
        "WriteOp",
        "McpCall",
        "session.db",
    ];

    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_else(|err| {
            panic!("failed to read {}: {err}", file.display());
        });
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "{} must not reference {needle}; MCP auditing belongs in the MITM endpoint/process, not the low-privilege aggregator subprocess",
                file.display()
            );
        }
    }
}

#[test]
fn response_call_result_roundtrip() {
    let resp = AggregatorResponse {
        id: 4,
        body: AggregatorResult::CallResult {
            result: serde_json::json!({"content": [{"type": "text", "text": "hello"}]}),
        },
    };
    let decoded = msgpack_roundtrip(&resp);
    if let AggregatorResult::CallResult { result } = decoded.body {
        assert_eq!(result["content"][0]["text"], "hello");
    } else {
        panic!("expected CallResult");
    }
}

#[tokio::test]
async fn async_frame_roundtrip_uses_the_real_wire_codec() {
    let request = AggregatorRequest {
        id: 77,
        method: AggregatorMethod::CallTool {
            name: "local__echo".into(),
            arguments: serde_json::json!({"message": "hello"}),
            timeout_ms: None,
        },
    };
    let (mut writer, mut reader) = tokio::io::duplex(4096);

    write_frame(&mut writer, &request).await.unwrap();
    let decoded: AggregatorRequest = read_frame(&mut reader).await.unwrap().unwrap();

    assert_eq!(decoded.id, 77);
    match decoded.method {
        AggregatorMethod::CallTool { name, arguments, .. } => {
            assert_eq!(name, "local__echo");
            assert_eq!(arguments["message"], "hello");
        }
        _ => panic!("expected call_tool request"),
    }
}

#[tokio::test]
async fn frame_reader_treats_clean_and_partial_length_eof_as_disconnects() {
    let (writer, mut reader) = tokio::io::duplex(16);
    drop(writer);
    let decoded: Option<AggregatorResponse> = read_frame(&mut reader).await.unwrap();
    assert!(decoded.is_none());

    let (mut writer, mut reader) = tokio::io::duplex(16);
    writer.write_all(&[0, 0]).await.unwrap();
    drop(writer);
    let decoded: Option<AggregatorResponse> = read_frame(&mut reader).await.unwrap();
    assert!(decoded.is_none());
}

#[tokio::test]
async fn frame_reader_rejects_oversized_truncated_and_invalid_payloads() {
    let (mut writer, mut reader) = tokio::io::duplex(32);
    writer.write_all(&(MAX_FRAME_SIZE + 1).to_be_bytes()).await.unwrap();
    let error = read_frame::<_, AggregatorResponse>(&mut reader)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("frame too large"), "unexpected error: {error}");

    let (mut writer, mut reader) = tokio::io::duplex(32);
    writer.write_all(&8_u32.to_be_bytes()).await.unwrap();
    writer.write_all(&[1, 2]).await.unwrap();
    drop(writer);
    let error = read_frame::<_, AggregatorResponse>(&mut reader)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("read frame payload"), "unexpected error: {error}");

    let (mut writer, mut reader) = tokio::io::duplex(32);
    writer.write_all(&1_u32.to_be_bytes()).await.unwrap();
    writer.write_all(&[0xc1]).await.unwrap();
    let error = read_frame::<_, AggregatorResponse>(&mut reader)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("msgpack deserialize"), "unexpected error: {error}");
}

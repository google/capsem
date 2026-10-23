use super::*;
use crate::ipc::file_boundary::FILE_SECURITY_CONTENT_PREVIEW_MAX;
use capsem_core::net::policy_config::{DetectionLevel, SecurityPluginConfig, SecurityPluginMode};
use capsem_proto::ipc::FileBoundaryAction;

async fn rewrite_reply(data: Vec<u8>, size: u64) -> ProcessToService {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(capsem_logger::DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
    let (dispatcher, _guest_control) = Dispatcher::with_db(temp.path(), db);
    *dispatcher.mcp_runtime.plugin_policy.write().unwrap() = Arc::new(BTreeMap::from([(
        "dummy_pre_eicar".into(),
        SecurityPluginConfig {
            content: Default::default(),
            mode: SecurityPluginMode::Rewrite,
            detection_level: DetectionLevel::High,
        },
    )]));
    let (request, response, handler) = channel(&dispatcher).await;
    request
        .send(ServiceToProcess::LogFileBoundary {
            id: 91,
            action: FileBoundaryAction::Import,
            path: "incoming.txt".into(),
            data,
            size,
            mime_type: Some("text/plain".into()),
        })
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(1), response.recv())
        .await
        .unwrap()
        .unwrap();
    handler.abort();
    reply
}

#[tokio::test]
async fn mutating_rewrite_refuses_incomplete_file_inspection() {
    let mut data = b"EICAR".to_vec();
    data.resize(FILE_SECURITY_CONTENT_PREVIEW_MAX + 1, b'x');
    let reply = rewrite_reply(data.clone(), data.len() as u64).await;
    assert!(
        matches!(
            reply,
            ProcessToService::LogFileBoundaryResult {
                success: false,
                data: None,
                error: Some(_),
                ..
            }
        ),
        "an incomplete preview cannot authorize replacement of the complete file: {reply:?}"
    );
}

#[tokio::test]
async fn mutating_rewrite_refuses_preview_only_and_lossy_binary_inputs() {
    for (data, size) in [(b"EICAR".to_vec(), 10), (b"EICAR\xff".to_vec(), 6)] {
        let reply = rewrite_reply(data, size).await;
        assert!(
            matches!(
                reply,
                ProcessToService::LogFileBoundaryResult {
                    success: false,
                    data: None,
                    error: Some(_),
                    ..
                }
            ),
            "incomplete or lossy input must fail closed: {reply:?}"
        );
    }
}

#[tokio::test]
async fn complete_utf8_rewrite_preserves_the_rest_of_the_file() {
    let data = "αEICAR🦊".as_bytes().to_vec();
    let reply = rewrite_reply(data.clone(), data.len() as u64).await;
    assert!(
        matches!(reply, ProcessToService::LogFileBoundaryResult {
        success: true, data: Some(ref bytes), error: None, ..
    } if bytes == "αCAPSEM_REWRITTEN_EICAR🦊".as_bytes()),
        "{reply:?}"
    );
}

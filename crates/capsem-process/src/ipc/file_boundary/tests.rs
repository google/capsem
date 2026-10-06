use crate::ipc::{tests::Dispatcher, *};
use std::collections::BTreeMap;

async fn channel(
    dispatcher: &Dispatcher,
) -> (
    Sender<ServiceToProcess>,
    Receiver<ProcessToService>,
    tokio::task::JoinHandle<Result<()>>,
) {
    let (owner, service) = tokio::net::UnixStream::pair().unwrap();
    let handler = dispatcher.connection(owner);
    let mut service = service.into_std().unwrap();
    let (request, response) = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service, "capsem-service-test", "").unwrap();
        channel_from_std::<ServiceToProcess, ProcessToService>(service).unwrap()
    })
    .await
    .unwrap();
    (request, response, handler)
}

fn import_request() -> ServiceToProcess {
    ServiceToProcess::LogFileBoundary {
        id: 91,
        action: capsem_proto::ipc::FileBoundaryAction::Import,
        path: ".capsem-image/options.json".into(),
        data: b"{}".to_vec(),
        size: 2,
        mime_type: Some("application/json".into()),
    }
}

#[tokio::test]
async fn host_file_boundary_answers_before_guest_readiness_without_control_consumption() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(capsem_logger::DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
    let (dispatcher, mut guest_control) = Dispatcher::with_db(temp.path(), Arc::clone(&db));
    dispatcher.ready.store(false, Ordering::Release);
    let (request, response, handler) = channel(&dispatcher).await;
    request.send(import_request()).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_millis(250), response.recv())
        .await
        .expect("host-only audit must not wait for the unconsumed guest control channel")
        .unwrap();
    assert!(
        matches!(
            reply,
            ProcessToService::LogFileBoundaryResult {
                id: 91,
                success: true,
                data: None,
                error: None,
            }
        ),
        "{reply:?}"
    );
    db.flush_checked().await.unwrap();
    let reader = db.reader().unwrap();
    let rows = reader.recent_file_events(10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, capsem_logger::FileAction::Imported);
    assert_eq!(rows[0].path, ".capsem-image/options.json");
    assert_eq!(rows[0].size, Some(2));
    assert!(rows[0].event_id.is_some());
    assert!(matches!(
        guest_control.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    assert!(!dispatcher.ready.load(Ordering::Acquire));
    handler.abort();
}

#[tokio::test]
async fn host_file_boundary_enforces_current_rules_and_records_denial_before_guest_ready() {
    use capsem_core::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(capsem_logger::DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
    let (dispatcher, mut guest_control) = Dispatcher::with_db(temp.path(), Arc::clone(&db));
    dispatcher.ready.store(false, Ordering::Release);
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.stop_import]
name = "stop_import"
action = "block"
detection_level = "high"
match = 'file.import.path.endsWith("options.json") && file.import.mime_type == "application/json"'
"#,
    )
    .unwrap();
    *dispatcher.mcp_runtime.security_rules.write().unwrap() =
        Arc::new(SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap());
    let (request, response, handler) = channel(&dispatcher).await;
    request.send(import_request()).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(1), response.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            reply,
            ProcessToService::LogFileBoundaryResult {
                id: 91,
                success: false,
                data: None,
                error: Some(_),
            }
        ),
        "{reply:?}"
    );
    db.flush_checked().await.unwrap();
    let reader = db.reader().unwrap();
    assert_eq!(reader.recent_file_events(10).unwrap().len(), 1);
    let detections = reader.recent_security_rule_events(10).unwrap();
    assert_eq!(detections.len(), 1);
    assert_eq!(detections[0].rule_id, "profiles.rules.stop_import");
    assert!(matches!(
        guest_control.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    handler.abort();
}

#[tokio::test]
async fn host_file_boundary_uses_live_preprocess_plugin_rewrites_before_guest_ready() {
    use capsem_core::net::policy_config::{
        DetectionLevel, SecurityPluginConfig, SecurityPluginMode, SecurityRuleProfile, SecurityRuleSet,
        SecurityRuleSource,
    };
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(capsem_logger::DbWriter::open(&temp.path().join("session.db"), 64).unwrap());
    let (dispatcher, mut guest_control) = Dispatcher::with_db(temp.path(), Arc::clone(&db));
    dispatcher.ready.store(false, Ordering::Release);
    let profile = SecurityRuleProfile::parse_toml(
        r#"
[profiles.rules.rewritten_import]
name = "rewritten_import"
action = "allow"
detection_level = "informational"
match = 'file.import.content == "CAPSEM_REWRITTEN_EICAR"'
"#,
    )
    .unwrap();
    *dispatcher.mcp_runtime.security_rules.write().unwrap() =
        Arc::new(SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap());
    *dispatcher.mcp_runtime.plugin_policy.write().unwrap() = Arc::new(BTreeMap::from([(
        "dummy_pre_eicar".into(),
        SecurityPluginConfig {
            mode: SecurityPluginMode::Rewrite,
            detection_level: DetectionLevel::High,
        },
    )]));
    let (request, response, handler) = channel(&dispatcher).await;
    let mut message = import_request();
    if let ServiceToProcess::LogFileBoundary { data, size, .. } = &mut message {
        *data = b"EICAR".to_vec();
        *size = 5;
    }
    request.send(message).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(1), response.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(reply, ProcessToService::LogFileBoundaryResult {
        id: 91, success: true, data: Some(ref bytes), error: None,
    } if bytes == b"CAPSEM_REWRITTEN_EICAR"),
        "{reply:?}"
    );
    db.flush_checked().await.unwrap();
    let reader = db.reader().unwrap();
    assert_eq!(reader.recent_file_events(10).unwrap().len(), 1);
    let matches = reader.recent_security_rule_events(10).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].rule_id, "profiles.rules.rewritten_import");
    assert!(matches!(
        guest_control.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    handler.abort();
}

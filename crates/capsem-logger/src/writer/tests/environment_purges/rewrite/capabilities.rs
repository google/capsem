use super::*;
use crate::db::EnvironmentHistoryPayload;
use capsem_config::environment::{EnvironmentDraft, EnvironmentSpecification};

const CAPABILITIES: &str = r#"
[image]
reference='development:stable'
resolved_digest='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
[startup]
script='printf approved-startup'
timeout_seconds=120
[secrets.api]
binding_id='00000000-0000-4000-8000-000000000001'
env_name='API_KEY'
[[repositories]]
url='https://example.org/team/project.git'
ref='main'
resolved_commit='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
destination='project'
[packages]
allows_package_manager=true
registries=['npm','pypi']
[access.custom.network]
name='Approved network'
action='ask'
match='http.host == "approved.example"'
priority=10
[[access.catalog.selected]]
id='registry-access'
version='1.0.0'
catalog_reference='ghcr.io/google/capsem/rules:stable'
resolved_digest='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
[access.parameters.registry-access]
port=443
[detection.custom.observe]
name='Observe network'
match='http.host == "approved.example"'
level='high'
[[skills.selected]]
id='sandbox'
version='1.0.0'
catalog_reference='ghcr.io/google/capsem/skills:stable'
resolved_digest='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
[mcps.servers.remote]
kind='remote'
url='https://mcp.example.org/rpc'
binding_id='00000000-0000-4000-8000-000000000001'
[mcps.servers.worker]
kind='sandbox'
executable='uvx'
args=['--offline','example-server']
workdir='/workspace'
[mcps.servers.worker.variables]
PROJECT='approved-launch-variable'
[mcps.permissions.worker]
default='block'
[mcps.permissions.worker.tools]
read='allow'
[plugins.pii_filter.inbound]
mode='rewrite'
[plugins.pii_filter.inbound.options]
email=false
[plugins.pii_filter.outbound]
mode='rewrite'
[plugins.pii_filter.outbound.options]
phone=false
[plugins.credential_broker.options]
capture_enabled=false
"#;

#[tokio::test]
async fn full_environment_capabilities_survive_archive_queries_and_variable_purge() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let source=format!("name='development'\ndescription='full capability fixture'\n[variables]\nWRONG='{SELECTED}'\nKEEP='unrelated value'\n{CAPABILITIES}");
    let full: EnvironmentSpecification = toml::from_str(&source).unwrap();
    full.validate().unwrap();
    let draft_source = source
        .lines()
        .filter(|line| !line.starts_with("resolved_") && !line.starts_with("catalog_reference="))
        .collect::<Vec<_>>()
        .join("\n");
    let draft: EnvironmentDraft = toml::from_str(&draft_source).unwrap();
    draft.validate().unwrap();
    let writer = DbWriter::open(&path, 8).unwrap();
    let mut first = revision(2, 3, None, Some(SELECTED));
    first.specification = Box::new(full.clone());
    committed(&writer, first).await;
    let mut clean = full.clone();
    clean.variables.remove("WRONG");
    let mut current = revision(2, 6, Some(3), None);
    current.specification = Box::new(clean.clone());
    committed(&writer, current).await;
    let payload = EnvironmentRequestPayload::Change {
        specification: Box::new(draft.clone()),
    };
    durable(
        &writer,
        WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: id(40),
            environment_id: id(2),
            base_revision_id: Some(id(6)),
            requester: None,
            timestamp_unix_ms: 40,
            expires_at_unix_ms: 10000,
            actor: "operator".into(),
            reason: "full capability intent".into(),
            payload_event_id: "000000000028".into(),
            payload: Box::new(payload.clone()),
        }),
    )
    .await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    assert_eq!(
        *reader
            .environment_revision_snapshot(&id(2), &id(3))
            .await
            .unwrap()
            .unwrap()
            .specification,
        full
    );
    assert_eq!(
        *reader
            .environment_request_snapshot(&id(40))
            .await
            .unwrap()
            .unwrap()
            .payload,
        payload
    );
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let purged = writer.purge_variable_history(execution(51)).await.unwrap();
    assert_eq!((purged.revision_count, purged.request_count), (1, 1));
    let history = reader
        .environment_revision_history(&id(2), &id(3))
        .await
        .unwrap()
        .unwrap();
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = history.payload
    else {
        panic!("partial revision")
    };
    assert_eq!(*value, clean, "every capability field must survive selective redaction");
    assert_eq!(purged_variables.len(), 1);
    let history = reader.environment_request_history(&id(40)).await.unwrap().unwrap();
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = history.payload
    else {
        panic!("partial request")
    };
    let mut clean_draft = draft;
    clean_draft.variables.remove("WRONG");
    assert_eq!(
        *value,
        EnvironmentRequestPayload::Change {
            specification: Box::new(clean_draft)
        }
    );
    assert_eq!(purged_variables.len(), 1);
    writer.shutdown_blocking();
    let restarted = crate::DbHandle::open_external_reader(&path).unwrap();
    assert_eq!(
        *restarted
            .environment_revision_snapshot(&id(2), &id(6))
            .await
            .unwrap()
            .unwrap()
            .specification,
        clean
    );
    assert!(restarted
        .environment_revision_snapshot(&id(2), &id(3))
        .await
        .unwrap_err()
        .contains("non-replayable"));
}

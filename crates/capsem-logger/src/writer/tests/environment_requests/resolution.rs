use super::*;

const RESOLVED: &str = r#"
name='development'
[image]
reference='development:stable'
resolved_digest='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
[[repositories]]
url='https://example.org/team/project.git'
ref='main'
resolved_commit='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
destination='project'
[[skills.selected]]
id='sandbox'
version='1.0.0'
catalog_reference='registry.example/skills:stable'
resolved_digest='sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'
[[access.catalog.selected]]
id='access-preset'
version='1.0.0'
catalog_reference='registry.example/rules:stable'
resolved_digest='sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'
[[detection.catalog.selected]]
id='observe'
version='1.0.0'
catalog_reference='registry.example/rules:stable'
resolved_digest='sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'
[[mcps.catalog.selected]]
id='server'
version='1.0.0'
catalog_reference='registry.example/mcps:stable'
resolved_digest='sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee'
"#;

#[tokio::test]
async fn request_resolution_pins_survive_native_archive_reopen_and_refuse_changed_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let mut request = proposal();
    request.payload = Box::new(EnvironmentRequestPayload::Change {
        specification: Box::new(toml::from_str(RESOLVED).expect("durable intent includes exact host pins")),
    });
    let writer = DbWriter::open(&path, 8).unwrap();
    writer
        .write_checked(WriteOp::PrepareEnvironmentRequest(request.clone()))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    writer.shutdown_blocking();
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let snapshot = reader
        .environment_request_snapshot(&request.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*snapshot.payload, *request.payload);
    let EnvironmentRequestPayload::Change { specification } = snapshot.payload.as_ref() else {
        panic!("change request")
    };
    let encoded = toml::to_string(specification.as_ref()).unwrap();
    for pin in [
        "a".repeat(64),
        "b".repeat(40),
        "c".repeat(64),
        "d".repeat(64),
        "e".repeat(64),
    ] {
        assert!(encoded.contains(&pin));
    }
    drop(reader);
    let writer = DbWriter::open(&path, 8).unwrap();
    writer
        .write_checked(WriteOp::PrepareEnvironmentRequest(request.clone()))
        .await
        .unwrap();
    writer.flush_checked().await.unwrap();
    let EnvironmentRequestPayload::Change { specification } = request.payload.as_mut() else {
        panic!("change request")
    };
    specification.image.as_mut().unwrap().resolved_digest = format!("sha256:{}", "f".repeat(64));
    rejected(&writer, WriteOp::PrepareEnvironmentRequest(request)).await;
}

#[test]
fn unresolved_environment_request_content_cannot_replace_reviewed_pins() {
    for removed in ["resolved_digest=", "resolved_commit=", "catalog_reference="] {
        let source = RESOLVED
            .lines()
            .filter(|line| !line.starts_with(removed))
            .collect::<Vec<_>>()
            .join("\n");
        let specification = toml::from_str::<capsem_config::environment::EnvironmentSpecification>(&source);
        assert!(specification.is_err(), "canonical request must retain {removed}");
    }
}

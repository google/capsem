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

#[tokio::test]
async fn approved_request_refuses_result_content_substitution_before_archive_publication() {
    for changed in [
        "image",
        "repository",
        "skill",
        "access",
        "detection",
        "mcp",
        "variable",
        "startup",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("host.db");
        let writer = DbWriter::open(&path, 8).unwrap();
        let mut request = proposal();
        request.payload = Box::new(EnvironmentRequestPayload::Change {
            specification: Box::new(toml::from_str(RESOLVED).unwrap()),
        });
        writer
            .write_checked(WriteOp::PrepareEnvironmentRequest(request.clone()))
            .await
            .unwrap();
        writer.flush_checked().await.unwrap();
        writer
            .write_checked(WriteOp::EnvironmentRequestDecision(decision(
                EnvironmentRequestDecisionKind::Approve,
            )))
            .await
            .unwrap();
        writer.flush_checked().await.unwrap();
        let EnvironmentRequestPayload::Change { specification } = request.payload.as_ref() else {
            unreachable!()
        };
        let mut result = revision(2, 3);
        result.request_id = Some(request.request_id);
        result.specification = specification.clone();
        let digest = format!("sha256:{}", "f".repeat(64));
        match changed {
            "image" => result.specification.image.as_mut().unwrap().resolved_digest = digest,
            "repository" => result.specification.repositories[0].resolved_commit = "f".repeat(40),
            "skill" => result.specification.skills.selected[0].resolved_digest = digest,
            "access" => result.specification.access.catalog.selected[0].resolved_digest = digest,
            "detection" => result.specification.detection.catalog.selected[0].resolved_digest = digest,
            "mcp" => result.specification.mcps.catalog.selected[0].resolved_digest = digest,
            "variable" => {
                result
                    .specification
                    .variables
                    .insert("UNREVIEWED".into(), "changed".into());
            }
            "startup" => {
                result.specification.startup = Some(capsem_config::environment::EnvironmentStartup {
                    script: "printf unreviewed".into(),
                    timeout_seconds: 30,
                })
            }
            _ => unreachable!(),
        }
        result.specification.validate().unwrap();
        rejected(&writer, WriteOp::PrepareEnvironmentRevision(result)).await;
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM environment_revisions", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM event_body_blobs", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn approved_exact_change_and_image_update_content_can_prepare_result() {
    for image_update in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let writer = DbWriter::open(&directory.path().join("host.db"), 8).unwrap();
        let mut request = proposal();
        request.timestamp_unix_ms = 30;
        let mut specification: capsem_config::environment::EnvironmentSpecification = toml::from_str(RESOLVED).unwrap();
        if image_update {
            let mut base = revision(2, 3);
            *base.specification = specification.clone();
            committed(&writer, &base).await;
            specification.image.as_mut().unwrap().resolved_digest = format!("sha256:{}", "f".repeat(64));
            request.base_revision_id = Some(id(3));
            request.payload = Box::new(EnvironmentRequestPayload::ImageUpdate {
                specification: Box::new(specification.clone()),
            });
        } else {
            request.payload = Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(specification.clone()),
            });
        }
        writer
            .write_checked(WriteOp::PrepareEnvironmentRequest(request))
            .await
            .unwrap();
        writer.flush_checked().await.unwrap();
        let mut approved = decision(EnvironmentRequestDecisionKind::Approve);
        approved.timestamp_unix_ms = 31;
        writer
            .write_checked(WriteOp::EnvironmentRequestDecision(approved))
            .await
            .unwrap();
        writer.flush_checked().await.unwrap();
        let mut result = revision(2, 4);
        result.timestamp_unix_ms = 40;
        result.request_id = Some(id(1));
        result.previous_revision_id = image_update.then(|| id(3));
        result.specification = Box::new(specification);
        writer
            .write_checked(WriteOp::PrepareEnvironmentRevision(result))
            .await
            .unwrap();
        writer.flush_checked().await.unwrap();
    }
}

#[tokio::test]
async fn promotion_result_requires_the_exact_immutable_source_content() {
    for substitute in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("host.db");
        let writer = DbWriter::open(&path, 8).unwrap();
        committed(&writer, &revision(2, 3)).await;
        let source = crate::events::EnvironmentRevisionParent {
            environment_id: id(6),
            revision_id: id(7),
        };
        let mut fork = revision(6, 7);
        fork.fork_parent = Some(crate::events::EnvironmentRevisionParent {
            environment_id: id(2),
            revision_id: id(3),
        });
        fork.specification = Box::new(toml::from_str(RESOLVED).unwrap());
        committed(&writer, &fork).await;
        let mut request = proposal();
        request.timestamp_unix_ms = 30;
        request.base_revision_id = Some(id(3));
        request.payload = Box::new(EnvironmentRequestPayload::Promotion { source });
        writer
            .write_checked(WriteOp::PrepareEnvironmentRequest(request))
            .await
            .unwrap();
        writer.flush_checked().await.unwrap();
        let mut approved = decision(EnvironmentRequestDecisionKind::Approve);
        approved.timestamp_unix_ms = 31;
        writer
            .write_checked(WriteOp::EnvironmentRequestDecision(approved))
            .await
            .unwrap();
        writer.flush_checked().await.unwrap();
        let mut result = revision(2, 8);
        result.timestamp_unix_ms = 40;
        result.previous_revision_id = Some(id(3));
        result.request_id = Some(id(1));
        result.specification = fork.specification;
        if substitute {
            result.specification.image.as_mut().unwrap().resolved_digest = format!("sha256:{}", "f".repeat(64));
            rejected(&writer, WriteOp::PrepareEnvironmentRevision(result)).await;
        } else {
            writer
                .write_checked(WriteOp::PrepareEnvironmentRevision(result))
                .await
                .unwrap();
            writer.flush_checked().await.unwrap();
        }
    }
}

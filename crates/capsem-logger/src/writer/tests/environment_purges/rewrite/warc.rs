use super::*;
use std::io::Read;

async fn export(reader: &crate::DbHandle, path: &Path) -> Vec<(String, Vec<u8>)> {
    let output = path.with_extension("warc.gz");
    let summary = reader
        .export_warc(std::fs::File::create(&output).unwrap())
        .await
        .unwrap();
    assert_eq!(summary.skipped_count, 0);
    let bytes = std::fs::read(output).unwrap();
    let mut remaining = bytes.as_slice();
    let mut resources = Vec::new();
    while !remaining.is_empty() {
        let mut decoder = flate2::bufread::GzDecoder::new(remaining);
        let mut record = Vec::new();
        decoder.read_to_end(&mut record).unwrap();
        remaining = decoder.into_inner();
        let split = record.windows(4).position(|bytes| bytes == b"\r\n\r\n").unwrap();
        let headers = String::from_utf8(record[..split].to_vec()).unwrap();
        if headers.lines().any(|line| line == "WARC-Type: resource") {
            assert!(headers.lines().any(|line| line == "WARC-Date: 1970-01-01T00:00:00Z"));
            let uri = headers
                .lines()
                .find_map(|line| line.strip_prefix("WARC-Target-URI: "))
                .unwrap();
            resources.push((uri.to_string(), record[split + 4..record.len() - 4].to_vec()));
        }
    }
    assert_eq!(summary.records as usize, resources.len());
    resources
}

#[tokio::test]
async fn warc_environment_exports_preserve_identity_and_mark_purged_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    let draft = toml::from_str(&format!(
        "name='development'\n[variables]\nWRONG='{SELECTED}'\nKEEP='unrelated value'\n"
    ))
    .unwrap();
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
            reason: "export fixture".into(),
            payload_event_id: "000000000028".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(draft),
            }),
        }),
    )
    .await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let before = export(&reader, &path).await;
    assert_eq!(before.len(), 6);
    let revision_uri = format!("capsem://environment-revision/{}/{}", id(2), id(3));
    let request_uri = format!("capsem://environment-request/{}/{}", id(2), id(40));
    for uri in [&revision_uri, &request_uri] {
        let body = &before.iter().find(|(key, _)| key == uri).unwrap().1;
        assert!(body.windows(SELECTED.len()).any(|bytes| bytes == SELECTED.as_bytes()));
    }
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    writer.purge_variable_history(execution(51)).await.unwrap();
    let after = export(&reader, &path).await;
    assert_eq!(after.len(), before.len());
    for uri in [&revision_uri, &request_uri] {
        let body = &after
            .iter()
            .find(|(key, _)| key == &format!("{uri}#non-replayable"))
            .unwrap()
            .1;
        assert!(!body.windows(SELECTED.len()).any(|bytes| bytes == SELECTED.as_bytes()));
        assert!(body
            .windows(b"unrelated value".len())
            .any(|bytes| bytes == b"unrelated value"));
    }
    let other_uri = format!("capsem://environment-revision/{}/{}", id(8), id(10));
    assert_eq!(
        before.iter().find(|(key, _)| key == &other_uri),
        after.iter().find(|(key, _)| key == &other_uri)
    );
}

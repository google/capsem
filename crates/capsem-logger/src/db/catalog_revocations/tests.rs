use super::*;
use crate::WriteOp;
use capsem_config::catalogs::CatalogKind;

fn record() -> CatalogRevocation {
    CatalogRevocation {
        key: CatalogRevocationKey {
            kind: CatalogKind::Skills,
            source_repository: "registry.example/team/catalog".into(),
            entry_id: "review".into(),
            version: "1.0.0".into(),
        },
        catalog_digest: format!("sha256:{}", "a".repeat(64)),
        observed_at_unix_ms: 10,
    }
}

#[tokio::test]
async fn catalog_revocation_survives_reopen_duplicates_and_later_observation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let db = DbHandle::open(&path).unwrap();
    let first = record();
    assert_eq!(db.catalog_revocation(first.key.clone()).await.unwrap(), None);
    db.write(WriteOp::ObserveCatalogRevocation(first.clone()))
        .await
        .unwrap();
    db.flush().await.unwrap();
    let mut later = first.clone();
    later.catalog_digest = format!("sha256:{}", "b".repeat(64));
    later.observed_at_unix_ms = 20;
    db.write(WriteOp::ObserveCatalogRevocation(later)).await.unwrap();
    db.write(WriteOp::ObserveCatalogRevocation(first.clone()))
        .await
        .unwrap();
    db.flush().await.unwrap();
    assert_eq!(
        db.catalog_revocation(first.key.clone()).await.unwrap(),
        Some(first.clone())
    );
    drop(db);
    let db = DbHandle::open(&path).unwrap();
    assert_eq!(
        db.catalog_revocation(first.key.clone()).await.unwrap(),
        Some(first.clone())
    );
    let mut out_of_order = first.clone();
    out_of_order.observed_at_unix_ms = 0;
    db.write(WriteOp::ObserveCatalogRevocation(out_of_order)).await.unwrap();
    db.flush().await.unwrap();
    assert_eq!(db.catalog_revocation(first.key.clone()).await.unwrap(), Some(first));
}

#[tokio::test]
async fn catalog_revocations_are_exactly_scoped_and_not_removed_by_retention() {
    let directory = tempfile::tempdir().unwrap();
    let db = DbHandle::open(&directory.path().join("host.db")).unwrap();
    let first = record();
    db.write(WriteOp::ObserveCatalogRevocation(first.clone()))
        .await
        .unwrap();
    db.flush().await.unwrap();
    for field in ["source", "kind", "entry", "version"] {
        let mut key = first.key.clone();
        match field {
            "source" => key.source_repository = "registry.example/other/catalog".into(),
            "kind" => key.kind = CatalogKind::Rules,
            "entry" => key.entry_id = "other".into(),
            _ => key.version = "2.0.0".into(),
        }
        assert_eq!(db.catalog_revocation(key).await.unwrap(), None);
    }
    db.retain_bodies_since("2100-01-01T00:00:00Z").await.unwrap();
    assert_eq!(db.catalog_revocation(first.key.clone()).await.unwrap(), Some(first));
}

#[tokio::test]
async fn invalid_catalog_revocations_are_rejected_without_replacing_valid_history() {
    let directory = tempfile::tempdir().unwrap();
    let db = DbHandle::open(&directory.path().join("host.db")).unwrap();
    let first = record();
    db.write(WriteOp::ObserveCatalogRevocation(first.clone()))
        .await
        .unwrap();
    db.flush().await.unwrap();
    for field in ["source", "entry", "version", "digest", "time"] {
        let mut invalid = first.clone();
        match field {
            "source" => invalid.key.source_repository = "registry.example/team/catalog:moving".into(),
            "entry" => invalid.key.entry_id = "../other".into(),
            "version" => invalid.key.version = "bad version".into(),
            "digest" => invalid.catalog_digest = "not-a-digest".into(),
            _ => invalid.observed_at_unix_ms = -1,
        }
        db.write(WriteOp::ObserveCatalogRevocation(invalid)).await.unwrap();
        assert!(db.flush().await.is_err(), "{field}");
    }
    db.flush().await.unwrap();
    assert_eq!(
        db.catalog_revocation(first.key.clone()).await.unwrap(),
        Some(first.clone())
    );
    db.write(WriteOp::ObserveCatalogRevocation(first.clone()))
        .await
        .unwrap();
    db.flush().await.unwrap();
    assert_eq!(db.catalog_revocation(first.key.clone()).await.unwrap(), Some(first));
}

#[tokio::test]
async fn missing_catalog_revocation_table_is_an_error_instead_of_no_revocations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let db = DbHandle::open(&path).unwrap();
    assert!(db.catalog_revocation(record().key).await.unwrap().is_none());
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE environment_catalog_revocations")
        .unwrap();
    assert!(db.catalog_revocation(record().key).await.is_err());
}

#[tokio::test]
async fn corrupt_catalog_revocation_provenance_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("host.db");
    let db = DbHandle::open(&path).unwrap();
    let first = record();
    db.write(WriteOp::ObserveCatalogRevocation(first.clone()))
        .await
        .unwrap();
    db.flush().await.unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA ignore_check_constraints=ON").unwrap();
    for digest in ["a".repeat(2048), format!("sha256:{}", "z".repeat(64))] {
        connection
            .execute(
                "UPDATE environment_catalog_revocations SET catalog_digest=?1",
                [&digest],
            )
            .unwrap();
        assert!(db.catalog_revocation(first.key.clone()).await.is_err());
    }
    connection
        .execute(
            "UPDATE environment_catalog_revocations SET catalog_digest=?1,observed_at_unix_ms=-1",
            [&first.catalog_digest],
        )
        .unwrap();
    assert!(db.catalog_revocation(first.key).await.is_err());
}

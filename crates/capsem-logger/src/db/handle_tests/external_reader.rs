//! What a service-side external reader is allowed to hold and to re-execute.
//!
//! The service reads ledgers that capsem-process writes. WAL already gives it
//! lock-free reads of the file, so it keeps no RAM mirror of the hot tables;
//! it queries `main` and caches whole `query_many` batches until SQLite's own
//! `data_version` says the writer committed.

use super::*;

/// A RAM mirror of every hot table of every running VM is what the service
/// used to pay for reads it can take straight from the file.
#[tokio::test]
async fn external_reader_attaches_no_memory_schema() {
    let p = temp_db_path("external-no-memory-schema");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    let mut event = make_net_event("external.example", Decision::Allowed);
    event.event_id = Some("0123456789ab".into());
    writer.write(WriteOp::NetEvent(event)).await.expect("write net event");
    writer.flush().await.expect("flush writer");

    let db = DbHandle::open_external_reader(&p).expect("open service external reader");
    db.ready().await.expect("external reader ready");

    let attached = db.attached_schemas_for_tests().await.expect("read attached schemas");
    assert!(
        !attached.iter().any(|name| name == crate::schema::MEMORY_SCHEMA),
        "an external reader must query the file through WAL, not a RAM mirror; attached {attached:?}. \
         {DB_BOUNDARY_RATIONALE}"
    );

    let rows = query_json(
        &db.query("SELECT event_id FROM net_events", &[])
            .await
            .expect("read row"),
    );
    assert_eq!(
        rows["rows"],
        json!([["0123456789ab"]]),
        "the disk-only reader must still resolve the hot tables. {DB_BOUNDARY_RATIONALE}"
    );
}

/// The polled aggregates (`stats/summary`, `security/status`) ran again on
/// every UI and TUI poll because the external handle skipped its own cache.
#[tokio::test]
async fn external_reader_caches_results_until_the_file_changes() {
    let p = temp_db_path("external-cache-follows-data-version");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    writer
        .write(WriteOp::NetEvent(make_net_event("cache.example", Decision::Allowed)))
        .await
        .expect("first write");
    writer.flush().await.expect("flush writer");

    let reader = DbHandle::open_external_reader(&p).expect("open service external reader");
    reader.ready().await.expect("external reader ready");

    let batch = || vec![("SELECT COUNT(*) AS n FROM net_events".to_string(), Vec::new())];
    // Both epochs are read after `ready()`, which already observed the ledger
    // once: comparing against zero would pass on that first bump alone.
    let epoch0 = reader.read_cache_epoch(ReadCacheDomain::All);
    let summary0 = reader.read_cache_epoch(ReadCacheDomain::SessionSummary);

    let first = reader.query_many(batch()).await.expect("first batch");
    assert_eq!(query_json(&first[0])["rows"], json!([[1]]));
    let executed_after_first = reader.queries_executed_for_tests().await.expect("read counter");

    let second = reader.query_many(batch()).await.expect("second batch");
    assert_eq!(second, first);
    assert_eq!(
        reader.queries_executed_for_tests().await.expect("read counter"),
        executed_after_first,
        "an unchanged ledger must be served from the handle cache without re-executing the SQL. \
         {DB_BOUNDARY_RATIONALE}"
    );
    assert_eq!(
        reader.read_cache_epoch(ReadCacheDomain::All),
        epoch0,
        "an unchanged ledger must not move the read epoch. {DB_BOUNDARY_RATIONALE}"
    );

    writer
        .write(WriteOp::NetEvent(make_net_event("cache.example", Decision::Allowed)))
        .await
        .expect("second write");
    writer.flush().await.expect("flush writer");

    let third = reader.query_many(batch()).await.expect("third batch");
    assert_eq!(
        query_json(&third[0])["rows"],
        json!([[2]]),
        "a commit by the writer must invalidate the cache. {DB_BOUNDARY_RATIONALE}"
    );
    assert!(
        reader.queries_executed_for_tests().await.expect("read counter") > executed_after_first,
        "a changed ledger must re-execute the batch. {DB_BOUNDARY_RATIONALE}"
    );
    assert!(
        reader.read_cache_epoch(ReadCacheDomain::All) > epoch0,
        "a commit observed by the external reader must move the read epoch so route caches \
         keyed on it expire. {DB_BOUNDARY_RATIONALE}"
    );
    assert!(
        reader.read_cache_epoch(ReadCacheDomain::SessionSummary) > summary0,
        "the session summary epoch is what ledger_routes keys its summary cache on. \
         {DB_BOUNDARY_RATIONALE}"
    );
}

/// An observation is a promise to redo everything derived from it, so it is
/// only recorded once that work succeeded.
///
/// Recording it up front loses the commit for good: the failed request has
/// already moved `synced_data_version` past it, so every later poll finds
/// nothing changed and answers from a cache built before those rows -- until
/// the writer happens to commit again, which for a finished VM is never.
#[tokio::test]
async fn a_failed_query_does_not_consume_the_commit_it_observed() {
    let p = temp_db_path("external-failed-query-keeps-the-commit");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    writer
        .write(WriteOp::NetEvent(make_net_event("retry.example", Decision::Allowed)))
        .await
        .expect("first write");
    writer.flush().await.expect("flush writer");

    let reader = DbHandle::open_external_reader(&p).expect("open service external reader");
    reader.ready().await.expect("external reader ready");
    let batch = || vec![("SELECT COUNT(*) AS n FROM net_events".to_string(), Vec::new())];
    assert_eq!(
        query_json(&reader.query_many(batch()).await.expect("first batch")[0])["rows"],
        json!([[1]])
    );

    writer
        .write(WriteOp::NetEvent(make_net_event("retry.example", Decision::Allowed)))
        .await
        .expect("second write");
    writer.flush().await.expect("flush writer");

    // This batch observes the commit and then fails on the second statement.
    let failed = reader
        .query_many(vec![
            ("SELECT COUNT(*) AS n FROM net_events".to_string(), Vec::new()),
            ("SELECT * FROM a_table_that_does_not_exist".to_string(), Vec::new()),
        ])
        .await;
    assert!(failed.is_err(), "the batch must fail: {failed:?}");

    let after = reader.query_many(batch()).await.expect("batch after the failure");
    assert_eq!(
        query_json(&after[0])["rows"],
        json!([[2]]),
        "a commit observed by a request that then failed must still invalidate the cache; \
         serving the pre-commit count here is stale until the writer commits again. \
         {DB_BOUNDARY_RATIONALE}"
    );
}

/// Route reads touch the file now, so they meet the file's locks.
///
/// An open write transaction is not one of them: WAL is what lets the reader
/// answer from the last committed snapshot instead of waiting. But
/// `checkpoint_and_vacuum_session_db` takes an exclusive lock that WAL does
/// not cover, and `retry_while_table_locked` only retries SQLITE_LOCKED on the
/// shared-cache schema, which a disk-only reader does not have. Without a
/// `busy_timeout` a route would fail with SQLITE_BUSY whenever a ledger
/// happened to be compacting.
#[tokio::test]
async fn an_external_read_waits_out_an_exclusive_file_lock() {
    let p = temp_db_path("external-waits-out-exclusive-lock");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    writer
        .write(WriteOp::NetEvent(make_net_event("locked.example", Decision::Allowed)))
        .await
        .expect("write net event");
    writer.flush().await.expect("flush writer");

    let reader = DbHandle::open_external_reader(&p).expect("open service external reader");
    reader.ready().await.expect("external reader ready");

    assert_eq!(
        reader.busy_timeout_ms_for_tests().await.expect("read busy_timeout"),
        5_000,
        "a disk-only reader must wait out an exclusive file lock -- a checkpoint or a vacuum -- \
         the same way the writer does, instead of failing a route with SQLITE_BUSY. \
         {DB_BOUNDARY_RATIONALE}"
    );

    let locker = rusqlite::Connection::open(&p).expect("second connection");
    locker
        .execute_batch(
            "BEGIN IMMEDIATE; INSERT INTO net_events (timestamp, domain, decision) \
             VALUES ('t', 'lock.example', 'allowed');",
        )
        .expect("hold an open write transaction");
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        locker.execute_batch("COMMIT").expect("release the transaction");
    });

    let rows = reader
        .query("SELECT COUNT(*) AS n FROM net_events", &[])
        .await
        .expect("a read must not fail while another connection holds an open write transaction");
    release.join().expect("locker thread");
    assert_eq!(
        query_json(&rows)["rows"],
        json!([[1]]),
        "WAL means the read answers from the last committed snapshot rather than waiting on or \
         seeing the open transaction. {DB_BOUNDARY_RATIONALE}"
    );
}

/// The disk-only reader still counts what it observes, so the poll path can be
/// measured: one `data_version` probe per batch, no copy.
#[tokio::test]
async fn external_reader_observes_no_change_on_an_idle_poll() {
    let p = temp_db_path("external-probe-per-batch");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    writer.flush().await.expect("flush writer");

    let reader = DbHandle::open_external_reader(&p).expect("open service external reader");
    reader.ready().await.expect("external reader ready");
    let batch = || vec![("SELECT COUNT(*) AS n FROM net_events".to_string(), Vec::new())];
    reader.query_many(batch()).await.expect("first batch");
    let syncs = reader.disk_syncs_for_tests().await.expect("read counter");
    reader.query_many(batch()).await.expect("second batch");
    assert_eq!(
        reader.disk_syncs_for_tests().await.expect("read counter"),
        syncs,
        "a poll with nothing committed must observe no change. {DB_BOUNDARY_RATIONALE}"
    );
}

/// Two polled routes share one session handle, so the batch cache has to hold
/// more than one batch to hold anything at all.
///
/// With a single slot, `stats/summary` and `security/status` evicted each
/// other on every poll: each one arrived, found the other's entry, missed,
/// re-executed, and stored over it. The cache existed and never hit once.
#[tokio::test]
async fn two_polled_batches_are_cached_side_by_side() {
    let p = temp_db_path("external-two-batches-side-by-side");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    writer
        .write(WriteOp::NetEvent(make_net_event("two.example", Decision::Allowed)))
        .await
        .expect("first write");
    writer.flush().await.expect("flush writer");

    let reader = DbHandle::open_external_reader(&p).expect("open service external reader");
    reader.ready().await.expect("external reader ready");

    // Two batches that no more resemble each other than the two routes do.
    let summary = || vec![("SELECT COUNT(*) AS n FROM net_events".to_string(), Vec::new())];
    let security = || {
        vec![
            (
                "SELECT COUNT(*) AS total FROM security_rule_events".to_string(),
                Vec::new(),
            ),
            (
                "SELECT rule_action, COUNT(*) FROM security_rule_events GROUP BY rule_action".to_string(),
                Vec::new(),
            ),
        ]
    };

    let first_summary = reader.query_many(summary()).await.expect("summary batch");
    let first_security = reader.query_many(security()).await.expect("security batch");
    let executed = reader.queries_executed_for_tests().await.expect("read counter");

    // A second poll of each, in the order the routes would arrive.
    assert_eq!(reader.query_many(summary()).await.expect("summary poll"), first_summary);
    assert_eq!(
        reader.query_many(security()).await.expect("security poll"),
        first_security
    );
    assert_eq!(
        reader.queries_executed_for_tests().await.expect("read counter"),
        executed,
        "both batches must survive the other's poll; a one-slot cache made each poll evict the \
         answer the next one was about to ask for. {DB_BOUNDARY_RATIONALE}"
    );

    writer
        .write(WriteOp::NetEvent(make_net_event("two.example", Decision::Allowed)))
        .await
        .expect("second write");
    writer.flush().await.expect("flush writer");

    // One commit expires the whole cache, not the entry that happens to be
    // looked up first: every entry was read from the ledger state it moved past.
    assert_eq!(
        query_json(&reader.query_many(summary()).await.expect("summary after commit")[0])["rows"],
        json!([[2]]),
        "a commit must invalidate the summary batch. {DB_BOUNDARY_RATIONALE}"
    );
    let executed_after_commit = reader.queries_executed_for_tests().await.expect("read counter");
    assert!(
        executed_after_commit > executed,
        "the summary batch must re-execute after a commit. {DB_BOUNDARY_RATIONALE}"
    );
    reader.query_many(security()).await.expect("security after commit");
    assert!(
        reader.queries_executed_for_tests().await.expect("read counter") > executed_after_commit,
        "the same commit must invalidate the security batch too, not only the batch that was \
         asked for first. {DB_BOUNDARY_RATIONALE}"
    );
}

/// The counter snapshot behind `stats/summary` and `/info` is a poll like any
/// other, so it is answered from the cache like any other.
///
/// The summary used to be a worker request of its own, which meant every poll
/// of every running session re-ran its aggregates over the file no matter how
/// long the ledger had stood still.
#[tokio::test]
async fn ledger_counters_are_served_from_the_batch_cache() {
    let p = temp_db_path("external-ledger-counters-cached");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    writer
        .write(WriteOp::NetEvent(make_net_event("stats.example", Decision::Allowed)))
        .await
        .expect("first write");
    writer.flush().await.expect("flush writer");

    let reader = DbHandle::open_external_reader(&p).expect("open service external reader");
    reader.ready().await.expect("external reader ready");

    // The counter has to move on the first call, or "it did not move on the
    // second" proves nothing: a private request that ran its query without
    // counting it would pass the cache assertion below while re-reading the
    // file on every poll.
    let before_first = reader.queries_executed_for_tests().await.expect("read counter");
    let first = reader.ledger_counters().await.expect("first counters");
    assert_eq!(first.net.total, 1);
    assert_eq!(first.net.allowed, 1);
    let executed = reader.queries_executed_for_tests().await.expect("read counter");
    assert!(
        executed > before_first,
        "the first counters poll must execute and count its query. {DB_BOUNDARY_RATIONALE}"
    );

    let second = reader.ledger_counters().await.expect("second counters");
    assert_eq!(second, first);
    assert_eq!(
        reader.queries_executed_for_tests().await.expect("read counter"),
        executed,
        "an unchanged ledger must answer a counters poll without re-reading the file. \
         {DB_BOUNDARY_RATIONALE}"
    );

    writer
        .write(WriteOp::NetEvent(make_net_event("stats.example", Decision::Denied)))
        .await
        .expect("second write");
    writer.flush().await.expect("flush writer");

    let third = reader.ledger_counters().await.expect("counters after commit");
    assert_eq!(third.net.total, 2, "a commit must be visible to the next poll");
    assert_eq!(third.net.denied, 1);
    assert!(
        reader.queries_executed_for_tests().await.expect("read counter") > executed,
        "a changed ledger must re-read the snapshot. {DB_BOUNDARY_RATIONALE}"
    );
}

/// A commit that lands between a poll's cache lookup and its reply costs at
/// most that one poll a stale answer; it must never be cached as current.
///
/// The two polled routes share one handle and run concurrently. Batch B finds
/// its old entry; batch A's worker observes the writer's commit and A expires
/// the cache; the worker then answers B "still valid", because the commit is
/// already recorded. If B read its epoch only after that -- as `query_many`
/// once did -- it stored its pre-commit answer under the post-commit epoch,
/// and an idle session served it until something else happened to commit.
#[tokio::test]
async fn a_commit_between_lookup_and_reply_is_never_cached_as_current() {
    let p = temp_db_path("external-lookup-race");
    let writer = DbHandle::open(&p).expect("open owning writer handle");
    writer
        .write(WriteOp::NetEvent(make_net_event("race.example", Decision::Allowed)))
        .await
        .expect("first write");
    writer.flush().await.expect("flush writer");

    let reader = DbHandle::open_external_reader(&p).expect("open service external reader");
    reader.ready().await.expect("external reader ready");

    let batch_b = || vec![("SELECT COUNT(*) AS n FROM net_events".to_string(), Vec::new())];
    let batch_a = || {
        vec![(
            "SELECT COUNT(*) AS total FROM security_rule_events".to_string(),
            Vec::new(),
        )]
    };
    let count = |raw: &[String]| query_json(&raw[0])["rows"][0][0].clone();

    assert_eq!(count(&reader.query_many(batch_b()).await.expect("prime B")), json!(1));

    writer
        .write(WriteOp::NetEvent(make_net_event("race.example", Decision::Allowed)))
        .await
        .expect("second write");
    writer.flush().await.expect("flush writer");

    // B looks its old entry up, then parks before asking the worker.
    let (parked, resume) = reader.pause_next_query_many_for_tests();
    let poll_b = tokio::spawn({
        let reader = reader.clone();
        async move { reader.query_many(batch_b()).await }
    });
    parked.await.expect("B parked after its lookup");

    // A observes the commit, records it, and expires the cache.
    reader.query_many(batch_a()).await.expect("A observes the commit");

    resume.send(()).expect("resume B");
    // B's own answer may be the old one: the race costs this one response.
    poll_b.await.expect("join B").expect("B completes");

    // It must not have been stored as current.
    assert_eq!(
        count(&reader.query_many(batch_b()).await.expect("next B poll")),
        json!(2),
        "a pre-commit answer was cached under the post-commit epoch and served again; \
         the epoch a result belongs to must be read with the lookup, before any \
         invalidation can land. {DB_BOUNDARY_RATIONALE}"
    );
}

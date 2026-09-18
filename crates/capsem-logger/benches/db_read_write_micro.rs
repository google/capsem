use std::ptr::NonNull;
use std::time::{Instant, SystemTime};

use capsem_logger::{schema, DbHandle, Decision, DnsEvent, WriteOp};
use memmap2::Mmap;
use rusqlite::{
    params,
    serialize::{Data, OwnedData},
    Connection, DatabaseName,
};

#[path = "support/net_bodies.rs"]
mod net_bodies;

const WRITE_ROWS: usize = 100_000;
const READ_ROWS: usize = 1_000_000;
/// Exchanges written before the cold read: about 900 KiB of raw bodies, just
/// under the archive's 1 MiB target, so the newest body sits at the far end
/// of the block that is still being written.
const ARCHIVE_EXCHANGES: usize = 140;
/// A disk flush every this many exchanges, standing in for the writer's
/// five-second timer on a live session.
const ARCHIVE_FLUSH_EVERY: usize = 10;

/// Timings of the archive path a polled route sees.
struct ArchiveTimings {
    write_ms: f64,
    cold_newest_ms: f64,
    warm_previous_ms: f64,
    cold_oldest_ms: f64,
}

/// Write a session's bodies with periodic flushes while the writer stays
/// open, then read them from a separate external handle -- the service's
/// view of a live `capsem-process` ledger.
///
/// `cold_newest` is the worst case for a route: a fresh reader asked for the
/// last body of the block still being appended, which inflates everything
/// before it in that block. `warm_previous` is the next row a UI asks for.
async fn archive_reads() -> ArchiveTimings {
    let dir = tempfile::tempdir().expect("temp db dir");
    let path = dir.path().join("session.db");
    let db = DbHandle::open(&path).expect("open db handle");
    db.ready().await.expect("db ready");

    let started = Instant::now();
    for idx in 0..ARCHIVE_EXCHANGES {
        db.write(net_bodies::net_event(idx)).await.expect("write net event");
        if idx % ARCHIVE_FLUSH_EVERY == ARCHIVE_FLUSH_EVERY - 1 {
            db.flush().await.expect("flush");
        }
    }
    db.flush().await.expect("final flush");
    let write_ms = started.elapsed().as_secs_f64() * 1000.0;

    // Median of five, each on a fresh reader: one cold read is a handful of
    // milliseconds and a single sample is mostly scheduler noise.
    let mut cold_newest = Vec::new();
    let mut warm_previous = Vec::new();
    let mut cold_oldest = Vec::new();
    for _ in 0..5 {
        let reader = DbHandle::open_external_reader(&path).expect("open external reader");
        reader.ready().await.expect("reader ready");
        cold_newest.push(timed_read(&reader, ARCHIVE_EXCHANGES - 1).await);
        warm_previous.push(timed_read(&reader, ARCHIVE_EXCHANGES - 2).await);
        let reader = DbHandle::open_external_reader(&path).expect("open external reader");
        reader.ready().await.expect("reader ready");
        cold_oldest.push(timed_read(&reader, 0).await);
    }
    drop(db);
    ArchiveTimings {
        write_ms,
        cold_newest_ms: median(cold_newest),
        warm_previous_ms: median(warm_previous),
        cold_oldest_ms: median(cold_oldest),
    }
}

async fn timed_read(reader: &DbHandle, idx: usize) -> f64 {
    let started = Instant::now();
    let body = reader
        .read_body(
            &net_bodies::event_id(idx),
            "net_events",
            capsem_logger::db::BodyDirection::Response,
        )
        .await
        .expect("read body")
        .expect("body is archived");
    assert!(!body.bytes.is_empty());
    started.elapsed().as_secs_f64() * 1000.0
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn dns_event(idx: usize) -> WriteOp {
    WriteOp::DnsEvent(DnsEvent {
        event_id: Some(format!("{:012x}", idx)),
        timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(idx as u64),
        qname: format!("bench-{idx}.example"),
        qtype: 1,
        qclass: 1,
        rcode: 0,
        answer_ip: Some("127.0.0.1".to_string()),
        decision: Decision::Allowed.as_str().to_string(),
        matched_rule: None,
        source_proto: Some("udp".to_string()),
        process_name: Some("bench".to_string()),
        upstream_resolver_ms: 0,
        trace_id: Some(format!("{idx:016x}")),
        policy_mode: None,
        policy_action: None,
        policy_rule: None,
        policy_reason: None,
        credential_ref: None,
    })
}

fn seed_dns_rows(path: &std::path::Path, rows: usize) {
    let mut conn = Connection::open(path).expect("open seed db");
    schema::apply_pragmas(&conn).expect("apply pragmas");
    schema::create_tables(&conn).expect("create schema");

    let tx = conn.transaction().expect("seed transaction");
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO dns_events (
                    event_id, timestamp, qname, qtype, qclass, rcode, answer_ip,
                    decision, source_proto, process_name, upstream_resolver_ms, trace_id, turn_id
                 )
                 VALUES (?1, ?2, ?3, 1, 1, 0, '127.0.0.1', 'allowed', 'udp', 'bench', 0, ?4, ?4)",
            )
            .expect("prepare seed insert");
        for idx in 0..rows {
            stmt.execute(params![
                format!("{:012x}", idx),
                "1970-01-01T00:00:00Z",
                format!("bench-{idx}.example"),
                format!("{idx:016x}"),
            ])
            .expect("insert seed row");
        }
    }
    tx.commit().expect("commit seed rows");
    conn.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE);
         PRAGMA journal_mode = DELETE;",
    )
    .expect("checkpoint seed rows before image deserialize");
}

fn sqlite_owned_data_from_mmap(path: &std::path::Path) -> OwnedData {
    let file = std::fs::File::open(path).expect("open db file for mmap");
    let mmap = unsafe { Mmap::map(&file).expect("mmap db file") };
    sqlite_owned_data_from_bytes(&mmap)
}

fn sqlite_owned_data_from_bytes(bytes: &[u8]) -> OwnedData {
    let len = bytes.len();
    assert!(len > 0, "cannot deserialize an empty SQLite file");
    let ptr = unsafe { rusqlite::ffi::sqlite3_malloc64(len as u64) } as *mut u8;
    let ptr = NonNull::new(ptr).expect("sqlite3_malloc64 returned null");
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), len);
        OwnedData::from_raw_nonnull(ptr, len)
    }
}

fn sqlite_owned_data_from_serialized(data: Data<'_>) -> OwnedData {
    sqlite_owned_data_from_bytes(&data)
}

async fn write_100k_rows() -> (f64, f64, f64) {
    let dir = tempfile::tempdir().expect("temp db dir");
    let path = dir.path().join("session.db");
    let db = DbHandle::open(&path).expect("open db handle");
    db.ready().await.expect("db ready");

    let started = Instant::now();
    for idx in 0..WRITE_ROWS {
        db.write(dns_event(idx)).await.expect("write dns row");
    }
    let ack_elapsed = started.elapsed();

    let shutdown_started = Instant::now();
    drop(db);
    let shutdown_elapsed = shutdown_started.elapsed();

    let query_started = Instant::now();
    let reopened = DbHandle::open(&path).expect("reopen db handle");
    reopened.ready().await.expect("reopened db ready");
    let count_json = reopened
        .query("SELECT COUNT(*) AS count FROM dns_events", &[])
        .await
        .expect("count written rows");
    let query_elapsed = query_started.elapsed();
    assert!(
        count_json.contains(&WRITE_ROWS.to_string()),
        "write bench count mismatch: {count_json}"
    );

    (
        ack_elapsed.as_secs_f64() * 1000.0,
        query_elapsed.as_secs_f64() * 1000.0,
        shutdown_elapsed.as_secs_f64() * 1000.0,
    )
}

async fn read_1m_rows() -> (f64, f64, f64, String) {
    let dir = tempfile::tempdir().expect("temp db dir");
    let path = dir.path().join("session.db");
    let seed_started = Instant::now();
    seed_dns_rows(&path, READ_ROWS);
    let seed_elapsed = seed_started.elapsed();

    let open_started = Instant::now();
    let db = DbHandle::open(&path).expect("open seeded db handle");
    db.ready().await.expect("seeded db ready");
    let open_elapsed = open_started.elapsed();

    let scan_started = Instant::now();
    let scan_json = db
        .query(
            "SELECT COUNT(*) AS count, SUM(qtype) AS qtype_sum, SUM(upstream_resolver_ms) AS upstream_sum FROM dns_events",
            &[],
        )
        .await
        .expect("scan 1m rows");
    let scan_elapsed = scan_started.elapsed();
    assert!(
        scan_json.contains(&READ_ROWS.to_string()),
        "read bench count mismatch: {scan_json}"
    );

    (
        seed_elapsed.as_secs_f64() * 1000.0,
        open_elapsed.as_secs_f64() * 1000.0,
        scan_elapsed.as_secs_f64() * 1000.0,
        scan_json,
    )
}

fn deserialize_scan_1m_rows() -> (f64, f64, String) {
    let dir = tempfile::tempdir().expect("temp db dir");
    let path = dir.path().join("session.db");
    seed_dns_rows(&path, READ_ROWS);

    let deserialize_started = Instant::now();
    let data = sqlite_owned_data_from_mmap(&path);
    let mut conn = Connection::open_in_memory().expect("open in-memory deserialize db");
    conn.deserialize(DatabaseName::Main, data, true)
        .expect("deserialize disk DB into memory");
    let deserialize_elapsed = deserialize_started.elapsed();

    let scan_started = Instant::now();
    let scan_json: (i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*) AS count, SUM(qtype) AS qtype_sum, SUM(upstream_resolver_ms) AS upstream_sum FROM dns_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("scan deserialized 1m rows");
    let scan_elapsed = scan_started.elapsed();
    assert_eq!(scan_json.0, READ_ROWS as i64, "deserialize scan count mismatch");

    (
        deserialize_elapsed.as_secs_f64() * 1000.0,
        scan_elapsed.as_secs_f64() * 1000.0,
        format!(
            r#"{{"columns":["count","qtype_sum","upstream_sum"],"rows":[[{},{},{}]]}}"#,
            scan_json.0, scan_json.1, scan_json.2
        ),
    )
}

fn serialize_deserialize_scan_1m_rows() -> (f64, f64, String) {
    let dir = tempfile::tempdir().expect("temp db dir");
    let path = dir.path().join("session.db");
    seed_dns_rows(&path, READ_ROWS);

    let deserialize_started = Instant::now();
    let source = Connection::open(&path).expect("open source db for sqlite serialize");
    let data = source.serialize(DatabaseName::Main).expect("serialize source DB");
    let data = sqlite_owned_data_from_serialized(data);
    let mut conn = Connection::open_in_memory().expect("open in-memory sqlite serialize db");
    conn.deserialize(DatabaseName::Main, data, true)
        .expect("deserialize sqlite serialized DB into memory");
    let deserialize_elapsed = deserialize_started.elapsed();

    let scan_started = Instant::now();
    let scan_json: (i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*) AS count, SUM(qtype) AS qtype_sum, SUM(upstream_resolver_ms) AS upstream_sum FROM dns_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("scan sqlite serialized 1m rows");
    let scan_elapsed = scan_started.elapsed();
    assert_eq!(scan_json.0, READ_ROWS as i64, "sqlite serialize scan count mismatch");

    (
        deserialize_elapsed.as_secs_f64() * 1000.0,
        scan_elapsed.as_secs_f64() * 1000.0,
        format!(
            r#"{{"columns":["count","qtype_sum","upstream_sum"],"rows":[[{},{},{}]]}}"#,
            scan_json.0, scan_json.1, scan_json.2
        ),
    )
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build benchmark runtime");

    let (write_ms, post_write_count_ms, shutdown_flush_ms) = rt.block_on(write_100k_rows());
    let (seed_ms, open_ms, scan_ms, scan_json) = rt.block_on(read_1m_rows());
    let (deserialize_ms, deserialize_scan_ms, deserialize_scan_json) = deserialize_scan_1m_rows();
    let (serialize_deserialize_ms, serialize_deserialize_scan_ms, serialize_deserialize_scan_json) =
        serialize_deserialize_scan_1m_rows();
    let archive = rt.block_on(archive_reads());

    println!("db read/write microbench");
    println!("| bench | rows | elapsed ms | rows/sec | notes |");
    println!("|---|---:|---:|---:|---|");
    println!(
        "| db_handle_write_accept_dns | {} | {:.3} | {:.0} | write() ack means accepted into the bounded DB-owned writer queue |",
        WRITE_ROWS,
        write_ms,
        WRITE_ROWS as f64 / (write_ms / 1000.0)
    );
    println!(
        "| db_handle_reopen_count_after_flush | {} | {:.3} | {:.0} | validates durable rows after shutdown flush and reopen |",
        WRITE_ROWS,
        post_write_count_ms,
        WRITE_ROWS as f64 / (post_write_count_ms / 1000.0)
    );
    println!(
        "| db_handle_drop_shutdown_flush | {} | {:.3} | {:.0} | flushes dirty memory rows to disk on close |",
        WRITE_ROWS,
        shutdown_flush_ms,
        WRITE_ROWS as f64 / (shutdown_flush_ms / 1000.0)
    );
    println!(
        "| seed_disk_for_read_setup | {} | {:.3} | {:.0} | setup only, not measured as DB contract |",
        READ_ROWS,
        seed_ms,
        READ_ROWS as f64 / (seed_ms / 1000.0)
    );
    println!(
        "| db_handle_open_1m | {} | {:.3} | {:.0} | open over 1M disk rows; nothing is copied into memory |",
        READ_ROWS,
        open_ms,
        READ_ROWS as f64 / (open_ms / 1000.0)
    );
    println!(
        "| db_handle_query_scan_1m | {} | {:.3} | {:.0} | aggregate scan through db.query(); result={} |",
        READ_ROWS,
        scan_ms,
        READ_ROWS as f64 / (scan_ms / 1000.0),
        scan_json.replace('|', "\\|")
    );
    println!(
        "| sqlite_deserialize_mmap_1m | {} | {:.3} | {:.0} | mmap + sqlite-owned copy + sqlite3_deserialize into in-memory main |",
        READ_ROWS,
        deserialize_ms,
        READ_ROWS as f64 / (deserialize_ms / 1000.0)
    );
    println!(
        "| sqlite_deserialize_query_scan_1m | {} | {:.3} | {:.0} | aggregate scan after deserialize; result={} |",
        READ_ROWS,
        deserialize_scan_ms,
        READ_ROWS as f64 / (deserialize_scan_ms / 1000.0),
        deserialize_scan_json.replace('|', "\\|")
    );
    println!(
        "| sqlite_serialize_deserialize_1m | {} | {:.3} | {:.0} | sqlite3_serialize from source DB + sqlite3_deserialize into in-memory main |",
        READ_ROWS,
        serialize_deserialize_ms,
        READ_ROWS as f64 / (serialize_deserialize_ms / 1000.0)
    );
    println!(
        "| sqlite_serialize_deserialize_query_scan_1m | {} | {:.3} | {:.0} | aggregate scan after sqlite serialize/deserialize; result={} |",
        READ_ROWS,
        serialize_deserialize_scan_ms,
        READ_ROWS as f64 / (serialize_deserialize_scan_ms / 1000.0),
        serialize_deserialize_scan_json.replace('|', "\\|")
    );
    println!(
        "| archive_write_net_bodies | {} | {:.3} | {:.0} | two bodies per exchange, a flush every {} |",
        ARCHIVE_EXCHANGES,
        archive.write_ms,
        ARCHIVE_EXCHANGES as f64 / (archive.write_ms / 1000.0),
        ARCHIVE_FLUSH_EVERY
    );
    println!(
        "| archive_cold_read_newest_body | 1 | {:.3} | - | fresh external reader, last body of the newest block |",
        archive.cold_newest_ms
    );
    println!(
        "| archive_warm_read_previous_body | 1 | {:.3} | - | same reader, the body before it |",
        archive.warm_previous_ms
    );
    println!(
        "| archive_cold_read_oldest_body | 1 | {:.3} | - | fresh external reader, first body of the archive |",
        archive.cold_oldest_ms
    );
}

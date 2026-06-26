use std::ptr::NonNull;
use std::time::{Instant, SystemTime};

use capsem_logger::{schema, DbHandle, Decision, DnsEvent, WriteOp};
use memmap2::Mmap;
use rusqlite::{
    params,
    serialize::{Data, OwnedData},
    Connection, DatabaseName,
};

const WRITE_ROWS: usize = 100_000;
const READ_ROWS: usize = 1_000_000;

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
    schema::migrate(&conn);

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
    assert_eq!(
        scan_json.0, READ_ROWS as i64,
        "deserialize scan count mismatch"
    );

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
    let data = source
        .serialize(DatabaseName::Main)
        .expect("serialize source DB");
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
    assert_eq!(
        scan_json.0, READ_ROWS as i64,
        "sqlite serialize scan count mismatch"
    );

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
    let (seed_ms, open_rehydrate_ms, scan_ms, scan_json) = rt.block_on(read_1m_rows());
    let (deserialize_ms, deserialize_scan_ms, deserialize_scan_json) = deserialize_scan_1m_rows();
    let (serialize_deserialize_ms, serialize_deserialize_scan_ms, serialize_deserialize_scan_json) =
        serialize_deserialize_scan_1m_rows();

    println!("db read/write microbench");
    println!("| bench | rows | elapsed ms | rows/sec | notes |");
    println!("|---|---:|---:|---:|---|");
    println!(
        "| db_handle_write_accept_dns | {} | {:.3} | {:.0} | write() ack means accepted into DB-owned producer buffer |",
        WRITE_ROWS,
        write_ms,
        WRITE_ROWS as f64 / (write_ms / 1000.0)
    );
    println!(
        "| db_handle_reopen_count_after_flush | {} | {:.3} | {:.0} | validates durable rows after shutdown flush and rehydrate |",
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
        "| db_handle_open_rehydrate_1m | {} | {:.3} | {:.0} | disk to DB-owned memory tables |",
        READ_ROWS,
        open_rehydrate_ms,
        READ_ROWS as f64 / (open_rehydrate_ms / 1000.0)
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
}

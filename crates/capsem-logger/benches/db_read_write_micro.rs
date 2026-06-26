use std::time::{Instant, SystemTime};

use capsem_logger::{schema, DbHandle, Decision, DnsEvent, WriteOp};
use rusqlite::{params, Connection};

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

    let query_started = Instant::now();
    let count_json = db
        .query("SELECT COUNT(*) AS count FROM dns_events", &[])
        .await
        .expect("count written rows");
    let query_elapsed = query_started.elapsed();
    assert!(
        count_json.contains(&WRITE_ROWS.to_string()),
        "write bench count mismatch: {count_json}"
    );

    let shutdown_started = Instant::now();
    drop(db);
    let shutdown_elapsed = shutdown_started.elapsed();

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

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build benchmark runtime");

    let (write_ms, post_write_count_ms, shutdown_flush_ms) = rt.block_on(write_100k_rows());
    let (seed_ms, open_rehydrate_ms, scan_ms, scan_json) = rt.block_on(read_1m_rows());

    println!("db read/write microbench");
    println!("| bench | rows | elapsed ms | rows/sec | notes |");
    println!("|---|---:|---:|---:|---|");
    println!(
        "| db_handle_write_ack_dns | {} | {:.3} | {:.0} | write() ack means memory-visible, not disk-flushed |",
        WRITE_ROWS,
        write_ms,
        WRITE_ROWS as f64 / (write_ms / 1000.0)
    );
    println!(
        "| db_handle_count_after_write | {} | {:.3} | {:.0} | validates db.query sees acknowledged memory rows |",
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
}

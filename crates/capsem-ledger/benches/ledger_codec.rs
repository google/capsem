use capsem_logger::{DbHandle, Decision, NetEvent, WriteOp};

#[path = "../src/codec/implementation.rs"]
mod codec;

const EXCHANGES: usize = 1_000;
const FLUSH_EVERY: usize = 20;

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("create benchmark runtime");
    for name in ["deflate", "zstd"] {
        runtime.block_on(measure(name));
    }
}

async fn measure(name: &str) {
    let dir = tempfile::tempdir().expect("create benchmark directory");
    let path = dir.path().join("session.db");
    let db = match name {
        "deflate" => DbHandle::open(&path).expect("open deflate ledger"),
        "zstd" => DbHandle::open_with_codecs(&path, codec::archive_codecs().expect("initialize zstd codec"))
            .expect("open zstd ledger"),
        _ => unreachable!(),
    };
    let baseline_rss = capsem_foundation::proctable::resident_bytes(std::process::id()).expect("read baseline RSS");
    let mut peak_rss = baseline_rss;
    let mut raw_bytes = 0usize;
    let started = std::time::Instant::now();
    for index in 0..EXCHANGES {
        let (event, bytes) = benchmark_event(index);
        raw_bytes += bytes;
        db.write(event).await.expect("write benchmark exchange");
        if index % FLUSH_EVERY == FLUSH_EVERY - 1 {
            db.flush().await.expect("flush benchmark ledger");
            peak_rss = peak_rss
                .max(capsem_foundation::proctable::resident_bytes(std::process::id()).expect("read benchmark RSS"));
        }
    }
    db.flush().await.expect("final benchmark flush");
    let elapsed = started.elapsed();
    drop(db);
    let archive_bytes: u64 = std::fs::read_dir(path.with_extension("bodies"))
        .expect("read archive generations")
        .map(|entry| {
            entry
                .expect("read archive generation")
                .metadata()
                .expect("read archive metadata")
                .len()
        })
        .sum();
    let seconds = elapsed.as_secs_f64();
    println!(
        "codec={name} exchanges={EXCHANGES} raw_bytes={raw_bytes} archive_bytes={archive_bytes} ratio={:.4} elapsed_ms={:.3} throughput_mib_s={:.3} peak_rss_delta_bytes={}",
        archive_bytes as f64 / raw_bytes as f64,
        seconds * 1000.0,
        raw_bytes as f64 / (1024.0 * 1024.0) / seconds,
        peak_rss.saturating_sub(baseline_rss),
    );
}

fn benchmark_event(index: usize) -> (WriteOp, usize) {
    let request = benchmark_text(index, 2048);
    let response = benchmark_text(index.wrapping_mul(31) + 7, 4096);
    let bytes = request.len() + response.len();
    (
        WriteOp::NetEvent(NetEvent {
            event_id: Some(format!("{index:012x}")),
            timestamp: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(index as u64),
            domain: "api.bench.example".into(),
            port: 443,
            decision: Decision::Allowed,
            process_name: Some("codec-bench".into()),
            pid: Some(1),
            method: Some("POST".into()),
            path: Some("/v1/messages".into()),
            query: None,
            status_code: Some(200),
            bytes_sent: request.len() as u64,
            bytes_received: response.len() as u64,
            duration_ms: 1,
            matched_rule: None,
            request_headers: Some("content-type: application/json".into()),
            response_headers: Some("content-type: application/json".into()),
            request_body: Some(request.into_bytes()),
            response_body: Some(response.into_bytes()),
            conn_type: Some("https".into()),
            policy_mode: None,
            policy_action: None,
            policy_rule: None,
            policy_reason: None,
            trace_id: Some(format!("{index:016x}")),
            credential_ref: None,
        }),
        bytes,
    )
}

fn benchmark_text(seed: usize, len: usize) -> String {
    const WORDS: [&str; 12] = [
        "assistant",
        "content",
        "stream",
        "delta",
        "tool",
        "result",
        "input",
        "output",
        "usage",
        "message",
        "role",
        "index",
    ];
    let mut state = (seed as u64).wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut output = String::with_capacity(len + 16);
    while output.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        output.push_str(WORDS[(state % WORDS.len() as u64) as usize]);
        output.push(if state.is_multiple_of(7) { ',' } else { ' ' });
    }
    output.truncate(len);
    output
}

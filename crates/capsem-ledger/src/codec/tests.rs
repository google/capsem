use capsem_archive::format::{BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, SEGMENT_HEADER_BYTES};
use capsem_archive::{BodyLogReader, BodyLogWriter, CODEC_ZSTD};
use capsem_logger::{DbHandle, Decision, NetEvent, WriteOp};

use super::archive_codecs;

#[test]
fn zstd_keeps_history_across_durable_segments_and_deflate_stays_readable() {
    let dir = tempfile::tempdir().unwrap();
    let zstd_path = dir.path().join("zstd.bodies");
    let codecs = archive_codecs().unwrap();
    let mut writer = BodyLogWriter::open_with_codecs(&zstd_path, codecs.clone()).unwrap();
    let body: Vec<u8> = (0_usize..8192)
        .map(|index| (index.wrapping_mul(37) & 0xff) as u8)
        .collect();
    let first = writer.stage(&body).unwrap();
    writer.flush_segment().unwrap();
    let before = writer.end();
    let second = writer.stage(&body).unwrap();
    writer.close_block().unwrap();
    assert!(
        writer.end() - before < 256,
        "the second segment did not reuse zstd history"
    );
    drop(writer);

    let encoded = std::fs::read(&zstd_path).unwrap();
    assert_eq!(encoded[FILE_HEADER_BYTES + 4], CODEC_ZSTD);
    let reader = BodyLogReader::open_with_codecs(&zstd_path, codecs.clone()).unwrap();
    assert_eq!(reader.read(first).unwrap(), body);
    assert_eq!(reader.read(second).unwrap(), body);

    let deflate_path = dir.path().join("deflate.bodies");
    let mut deflate = BodyLogWriter::open(&deflate_path).unwrap();
    let reference = deflate.stage(b"an archive from before zstd").unwrap();
    deflate.close_block().unwrap();
    let compatible = BodyLogReader::open_with_codecs(&deflate_path, codecs).unwrap();
    assert_eq!(compatible.read(reference).unwrap(), b"an archive from before zstd");
}

#[test]
fn zstd_corruption_is_refused_before_bytes_are_returned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.bodies");
    let codecs = archive_codecs().unwrap();
    let mut writer = BodyLogWriter::open_with_codecs(&path, codecs.clone()).unwrap();
    let reference = writer.stage(b"authenticated zstd bytes").unwrap();
    writer.close_block().unwrap();
    drop(writer);

    let mut encoded = std::fs::read(&path).unwrap();
    let payload = FILE_HEADER_BYTES + BLOCK_HEADER_BYTES + SEGMENT_HEADER_BYTES;
    encoded[payload + 1] ^= 0x80;
    std::fs::write(&path, encoded).unwrap();
    let reader = BodyLogReader::open_with_codecs(&path, codecs).unwrap();
    assert!(reader.read(reference).is_err());
}

#[tokio::test]
#[ignore = "repeatable manual codec measurement"]
async fn measure_identical_ledger_codec() {
    const EXCHANGES: usize = 1_000;
    const FLUSH_EVERY: usize = 20;
    let codec = std::env::var("CAPSEM_CODEC_BENCH").expect("CAPSEM_CODEC_BENCH=deflate|zstd");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = match codec.as_str() {
        "deflate" => DbHandle::open(&path).unwrap(),
        "zstd" => DbHandle::open_with_codecs(&path, archive_codecs().unwrap()).unwrap(),
        other => panic!("unsupported codec {other}"),
    };
    let baseline_rss = capsem_foundation::proctable::resident_bytes(std::process::id()).unwrap();
    let mut peak_rss = baseline_rss;
    let mut raw_bytes = 0usize;
    let started = std::time::Instant::now();
    for index in 0..EXCHANGES {
        let (event, bytes) = benchmark_event(index);
        raw_bytes += bytes;
        db.write(event).await.unwrap();
        if index % FLUSH_EVERY == FLUSH_EVERY - 1 {
            db.flush().await.unwrap();
            peak_rss = peak_rss.max(capsem_foundation::proctable::resident_bytes(std::process::id()).unwrap());
        }
    }
    db.flush().await.unwrap();
    let elapsed = started.elapsed();
    drop(db);
    let archive_bytes: u64 = std::fs::read_dir(path.with_extension("bodies"))
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    let seconds = elapsed.as_secs_f64();
    eprintln!(
        "codec={codec} exchanges={EXCHANGES} raw_bytes={raw_bytes} archive_bytes={archive_bytes} ratio={:.4} elapsed_ms={:.3} throughput_mib_s={:.3} peak_rss_delta_bytes={}",
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

//! What an exec leaves in the session ledger, read back the way a route reads
//! it: through the DB handle, after the writer's flush barrier.
//!
//! The ledger used to archive the 1 KiB display preview as the exec's output,
//! so a body read of any longer command returned its first kilobyte and a
//! truncation flag -- the rest had never been admitted.

use super::*;
use capsem_logger::{BodyDirection, DbHandle, ExecEvent, WriteOp};
use capsem_proto::ExecOutputChannel;

use super::super::super::exec_output::{self, ExecCapture, EXEC_LEDGER_BODY_BYTES, MAX_EXEC_OUTPUT_BYTES};

struct Ledger {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    db: Arc<capsem_logger::DbWriter>,
    js: Arc<JobStore>,
}

impl Ledger {
    fn open() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.db");
        let db = Arc::new(capsem_logger::DbWriter::open(&path, 16).unwrap());
        Self {
            _dir: dir,
            path,
            db,
            js: Arc::new(JobStore::new()),
        }
    }

    /// Start exec `id` the way the service does: its start row, keyed on the
    /// event id the completion's bodies will hang from.
    async fn start(&self, id: u64, event_id: &str, stream: Option<mpsc::Sender<capsem_proto::ipc::ProcessToService>>) {
        self.db
            .write(WriteOp::ExecEvent(ExecEvent {
                event_id: Some(event_id.to_string()),
                timestamp: std::time::SystemTime::now(),
                exec_id: id,
                command: "produce output".into(),
                source: "api".into(),
                trace_id: None,
                process_name: None,
                credential_ref: None,
            }))
            .await;
        let mut active = ActiveExec::new();
        active.event_id = Some(capsem_core::security_engine::SecurityEventId::parse(event_id).unwrap());
        active.stream = stream;
        self.js.active_execs.lock().unwrap().insert(id, active);
    }

    /// Run the guest side: its output through the real reader, then ExecDone.
    async fn finish(&self, id: u64, capture: ExecCapture) -> JobResult {
        let (tx, result) = oneshot::channel();
        self.js.jobs.lock().unwrap().insert(id, tx);
        exec_output::deposit(&self.js, id, capture).unwrap().notify_one();
        let rules = Arc::new(std::sync::RwLock::new(Arc::new(
            capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
        )));
        let plugins = Arc::new(std::sync::RwLock::new(Arc::new(std::collections::BTreeMap::new())));
        super::super::super::handle_guest_msg(
            GuestToHost::ExecDone { id, exit_code: 0 },
            &self.js,
            &self.db,
            &rules,
            &plugins,
        )
        .await;
        result.await.unwrap()
    }

    async fn body(&self, event_id: &str, direction: BodyDirection) -> Option<capsem_logger::StoredBody> {
        self.db.flush().await;
        let handle = DbHandle::open_external_reader(&self.path).unwrap();
        handle.ready().await.unwrap();
        handle.read_body(event_id, "exec_events", direction).await.unwrap()
    }

    async fn previews(&self, id: u64) -> serde_json::Value {
        self.db.flush().await;
        let handle = DbHandle::open_external_reader(&self.path).unwrap();
        let raw = handle
            .query(
                "SELECT stdout_preview, stderr_preview, stdout_bytes, stderr_bytes FROM exec_events WHERE exec_id = ?",
                &[serde_json::json!(id)],
            )
            .await
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&raw).unwrap()["rows"][0].clone()
    }
}

fn framed(frames: &[(ExecOutputChannel, &[u8])]) -> Vec<u8> {
    let mut wire = Vec::new();
    for (channel, data) in frames {
        for chunk in data.chunks(capsem_proto::MAX_EXEC_DATA_BYTES) {
            capsem_proto::write_exec_output(
                &mut wire,
                &capsem_proto::ExecOutputFrame {
                    channel: *channel,
                    data: chunk.to_vec(),
                },
            )
            .unwrap();
        }
    }
    wire
}

fn assert_archived(body: &capsem_logger::StoredBody, expected: &[u8], original: u64) {
    assert_eq!(body.bytes.len(), expected.len(), "stored length");
    assert!(body.bytes == expected, "stored bytes differ from what the guest wrote");
    // `read_body` checks the bytes against this hash and fails the read on a
    // mismatch, so a read that returned is a hash that held.
    assert!(body.body_hash.starts_with("blake3:"), "{}", body.body_hash);
    assert_eq!(body.original_bytes, original);
    assert_eq!(body.truncated, original > expected.len() as u64);
}

/// Deterministic, not UTF-8, and longer than any preview.
fn guest_bytes(len: usize, salt: u8) -> Vec<u8> {
    (0..len)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(salt) | 0x80)
        .collect()
}

#[tokio::test]
async fn buffered_output_is_archived_whole_on_both_lanes() {
    let ledger = Ledger::open();
    let stdout = guest_bytes(40_000, 1);
    let stderr = guest_bytes(9_000, 2);
    ledger.start(1, "00000000e0a1", None).await;
    let capture = exec_output::read_exec_output(&mut std::io::Cursor::new(framed(&[
        (ExecOutputChannel::Stdout, &stdout[..10_000]),
        (ExecOutputChannel::Stderr, &stderr),
        (ExecOutputChannel::Stdout, &stdout[10_000..]),
    ])));

    let JobResult::Exec {
        stdout: returned,
        stderr: returned_err,
        truncated,
        ..
    } = ledger.finish(1, capture).await
    else {
        panic!("exec failed")
    };
    assert!(
        returned == stdout && returned_err == stderr && !truncated,
        "the result is unchanged"
    );

    let out = ledger
        .body("00000000e0a1", BodyDirection::Stdout)
        .await
        .expect("stdout archived");
    assert_archived(&out, &stdout, stdout.len() as u64);
    let err = ledger
        .body("00000000e0a1", BodyDirection::Stderr)
        .await
        .expect("stderr archived");
    assert_archived(&err, &stderr, stderr.len() as u64);

    // The preview is still a preview: bounded, derived from the body.
    let row = ledger.previews(1).await;
    let preview = row[0].as_str().unwrap();
    assert!(preview.len() <= 2048, "preview is {} bytes", preview.len());
    assert_eq!(preview, capsem_logger::output_preview(&stdout));
    assert_eq!(row[2], serde_json::json!(stdout.len()));
    assert_eq!(row[3], serde_json::json!(stderr.len()));
}

/// Output past one result's budget: the result is cut, as before, but the
/// ledger keeps each lane to its own cap -- and marks the lane truncated only
/// where its own cap cut it.
#[tokio::test]
async fn the_result_cap_does_not_cut_the_ledger() {
    let ledger = Ledger::open();
    let stdout = guest_bytes(EXEC_LEDGER_BODY_BYTES + 3, 3);
    let stderr = guest_bytes(1_500_000, 4);
    ledger.start(2, "00000000e0a2", None).await;
    let capture = exec_output::read_exec_output(&mut std::io::Cursor::new(framed(&[
        (ExecOutputChannel::Stderr, &stderr),
        (ExecOutputChannel::Stdout, &stdout),
    ])));

    let JobResult::Exec {
        stdout: returned,
        stderr: returned_err,
        truncated,
        ..
    } = ledger.finish(2, capture).await
    else {
        panic!("exec failed")
    };
    assert!(truncated, "the result says it was cut");
    assert_eq!(returned_err.len(), stderr.len(), "stderr arrived first and is whole");
    assert_eq!(returned.len(), MAX_EXEC_OUTPUT_BYTES - stderr.len());
    assert!(returned[..] == stdout[..returned.len()]);

    let out = ledger
        .body("00000000e0a2", BodyDirection::Stdout)
        .await
        .expect("stdout archived");
    assert_archived(&out, &stdout[..EXEC_LEDGER_BODY_BYTES], stdout.len() as u64);
    assert!(out.truncated, "three bytes past the cap were cut, and it says so");
    let err = ledger
        .body("00000000e0a2", BodyDirection::Stderr)
        .await
        .expect("stderr archived");
    assert_archived(&err, &stderr, stderr.len() as u64);
    assert!(!err.truncated);
}

/// A streamed exec's bytes went to its client, and the ledger used to keep
/// 1 KiB of them. Returning output to an SDK is not archiving it.
#[tokio::test]
async fn streamed_output_is_archived_whole() {
    let ledger = Ledger::open();
    let stdout = guest_bytes(200_000, 5);
    let stderr = guest_bytes(3_000, 6);
    let (sender, mut client) = mpsc::channel(64);
    ledger.start(3, "00000000e0a3", Some(sender.clone())).await;
    let wire = framed(&[
        (ExecOutputChannel::Stdout, &stdout),
        (ExecOutputChannel::Stderr, &stderr),
    ]);
    let reader =
        std::thread::spawn(move || exec_output::stream_exec_output(&mut std::io::Cursor::new(wire), 3, &sender));
    let mut delivered = 0;
    while delivered < stdout.len() + stderr.len() {
        if let Some(capsem_proto::ipc::ProcessToService::ExecOutput { data, .. }) = client.recv().await {
            delivered += data.len();
        }
    }
    let capture = reader.join().unwrap();

    let JobResult::Exec { stdout: returned, .. } = ledger.finish(3, capture).await else {
        panic!("exec failed")
    };
    assert!(
        returned.is_empty(),
        "a stream's result does not replay what it delivered"
    );
    let out = ledger
        .body("00000000e0a3", BodyDirection::Stdout)
        .await
        .expect("stdout archived");
    assert_archived(&out, &stdout, stdout.len() as u64);
    let err = ledger
        .body("00000000e0a3", BodyDirection::Stderr)
        .await
        .expect("stderr archived");
    assert_archived(&err, &stderr, stderr.len() as u64);
}

/// A client that goes away cancels its stream. The guest is still drained and
/// what it wrote is still evidence: the ledger keeps it, untruncated, even
/// though the result reports the disconnect.
#[tokio::test]
async fn a_cancelled_stream_still_archives_its_output() {
    let ledger = Ledger::open();
    let stdout = guest_bytes(120_000, 7);
    let (sender, client) = mpsc::channel(1);
    ledger.start(4, "00000000e0a4", Some(sender.clone())).await;
    drop(client);
    let wire = framed(&[(ExecOutputChannel::Stdout, &stdout)]);
    let capture =
        std::thread::spawn(move || exec_output::stream_exec_output(&mut std::io::Cursor::new(wire), 4, &sender))
            .join()
            .unwrap();

    match ledger.finish(4, capture).await {
        JobResult::Error { message } => assert!(message.contains("disconnected"), "{message}"),
        other => panic!("a cancelled stream reported {other:?}"),
    }
    let out = ledger
        .body("00000000e0a4", BodyDirection::Stdout)
        .await
        .expect("stdout archived");
    assert_archived(&out, &stdout, stdout.len() as u64);
}

/// Output cut short by a broken transport is archived as far as it got, and
/// not called truncated: nothing the capture received was dropped.
#[tokio::test]
async fn output_that_stops_on_a_broken_frame_is_archived_as_far_as_it_got() {
    let ledger = Ledger::open();
    let stdout = guest_bytes(5_000, 8);
    ledger.start(5, "00000000e0a5", None).await;
    let mut wire = framed(&[(ExecOutputChannel::Stdout, &stdout)]);
    wire.extend_from_slice(&[0, 0, 0, 12, 1, 2]);
    let capture = exec_output::read_exec_output(&mut std::io::Cursor::new(wire));

    assert!(matches!(ledger.finish(5, capture).await, JobResult::Error { .. }));
    let out = ledger
        .body("00000000e0a5", BodyDirection::Stdout)
        .await
        .expect("stdout archived");
    assert_archived(&out, &stdout, stdout.len() as u64);
}

/// Rules are matched against every completion, so their input is the bounded
/// preview, not the archived body.
#[test]
fn rule_input_is_the_bounded_preview_not_the_body() {
    let body = guest_bytes(EXEC_LEDGER_BODY_BYTES, 9);
    let event =
        capsem_core::security_engine::security_event_from_exec_complete_event(&capsem_logger::ExecEventComplete {
            exec_id: 6,
            exit_code: 0,
            duration_ms: 1,
            stdout: body.clone(),
            stderr: body.clone(),
            stdout_bytes: body.len() as u64,
            stderr_bytes: body.len() as u64,
            pid: None,
        });
    let process = event.process.expect("process fields");
    let stdout = process.stdout.expect("stdout is rule input");
    assert!(stdout.len() <= 2048, "rule input is {} bytes", stdout.len());
    assert_eq!(stdout, capsem_logger::output_preview(&body));
}

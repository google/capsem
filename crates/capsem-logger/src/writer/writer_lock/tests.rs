use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use super::*;
use crate::DbWriter;

/// Set only in the child the cross-process test spawns: the ledger it holds.
const HOLDER_ENV: &str = "CAPSEM_TEST_LEDGER_WRITER_HOLDER";
const HOLDER_TEST: &str = "writer::writer_lock::tests::holds_a_ledger_writer_for_another_process";
const HOLDING: &str = "capsem-ledger-writer-holding";

fn refusal(result: rusqlite::Result<DbWriter>) -> String {
    match result {
        Ok(_) => panic!("a second writer on the same ledger must be refused at open"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn the_sidecar_sits_beside_the_ledger() {
    assert_eq!(
        writer_lock_path(Path::new("/s/session.db")),
        PathBuf::from("/s/session.db-writer.lock")
    );
}

#[test]
fn a_second_writer_in_the_same_process_is_refused_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let first = DbWriter::open(&db_path, 8).unwrap();

    let message = refusal(DbWriter::open(&db_path, 8));
    assert!(
        message.contains(&db_path.display().to_string()),
        "the refusal names the ledger: {message}"
    );
    assert!(message.contains("already has a writer"), "{message}");

    drop(first);
    DbWriter::open(&db_path, 8).expect("the lock is released when the first writer's thread ends");
}

/// Shutting the writer down releases the ledger even while the handle lives:
/// the lock belongs to the writer thread, not to the value.
#[test]
fn a_shut_down_writer_no_longer_holds_the_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let first = DbWriter::open(&db_path, 8).unwrap();
    first.shutdown_blocking();

    let _second = DbWriter::open(&db_path, 8).expect("a writer whose thread has joined holds nothing");
    drop(first);
}

/// Not a test on its own: the child half of the cross-process test below.
/// It does nothing unless that test spawned it.
#[test]
fn holds_a_ledger_writer_for_another_process() {
    let Some(db_path) = std::env::var_os(HOLDER_ENV) else {
        return;
    };
    let _writer = DbWriter::open(Path::new(&db_path), 8).expect("the holder opens the ledger");
    println!("{HOLDING}");
    std::io::stdout().flush().unwrap();
    // Hold it until the parent closes our stdin.
    let _ = std::io::stdin().read_line(&mut String::new());
}

/// The case that corrupted the archive: two processes, one ledger.
#[test]
fn a_second_process_cannot_open_a_writer_on_the_same_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args([HOLDER_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(HOLDER_ENV, &db_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the holding process");
    // Kept open to the end: the holder's harness still prints after it lets go.
    let mut lines = BufReader::new(holder.stdout.take().unwrap())
        .lines()
        .map_while(Result::ok);
    assert!(
        lines.any(|line| line.contains(HOLDING)),
        "the holder never reported holding the ledger"
    );

    let message = refusal(DbWriter::open(&db_path, 8));
    assert!(
        message.contains(&db_path.display().to_string()),
        "the refusal names the ledger: {message}"
    );

    drop(holder.stdin.take());
    lines.for_each(drop);
    assert!(holder.wait().unwrap().success(), "the holder exits cleanly");
    DbWriter::open(&db_path, 8).expect("the ledger is free once its writer's process is gone");
}

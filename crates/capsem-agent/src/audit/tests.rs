use std::collections::HashMap;
use std::io::Cursor;

use super::*;

fn group(id: u32, exe: &str) -> String {
    format!(
        "type=SYSCALL msg=audit(1713100000.001:{id}): arch=c000003e syscall=59 success=yes exit=0 ppid=100 pid=200 auid=1000 uid=0 gid=0 tty=pts0 comm=\"x\" exe=\"{exe}\" key=\"exec\"\n\
         type=EXECVE msg=audit(1713100000.001:{id}): argc=1 a0=\"{exe}\"\n\
         type=CWD msg=audit(1713100000.001:{id}): cwd=\"/root\"\n\
         type=PROCTITLE msg=audit(1713100000.001:{id}): proctitle=\"{exe}\"\n"
    )
}

fn decode(frame: &[u8]) -> AuditRecord {
    capsem_proto::decode_audit_record(&frame[4..]).unwrap()
}

#[test]
fn complete_groups_are_emitted_once_each() {
    let mut reader = Cursor::new(format!("{}{}", group(1, "/bin/first"), group(2, "/bin/second")));
    let mut pending = HashMap::new();
    let mut frames = Vec::new();
    tail_audit_log(&mut reader, &mut pending, &mut |frame| {
        frames.push(frame.to_vec());
        true
    })
    .unwrap();
    let exes: Vec<String> = frames.iter().map(|f| decode(f).exe).collect();
    assert_eq!(exes, vec!["/bin/first", "/bin/second"]);
    assert!(pending.is_empty());
}

// The tailer used to be respawned per host connection, so every reconnect
// re-streamed the log from the top. Now a refused frame comes back to the
// caller and the next call continues from the current offset.
#[test]
fn a_refused_frame_is_returned_and_the_log_is_not_rewound() {
    let mut reader = Cursor::new(format!("{}{}", group(1, "/bin/first"), group(2, "/bin/second")));
    let mut pending = HashMap::new();
    let refused = tail_audit_log(&mut reader, &mut pending, &mut |_| false).unwrap_err();
    assert_eq!(decode(&refused).exe, "/bin/first");

    let mut frames = Vec::new();
    tail_audit_log(&mut reader, &mut pending, &mut |frame| {
        frames.push(frame.to_vec());
        true
    })
    .unwrap();
    let exes: Vec<String> = frames.iter().map(|f| decode(f).exe).collect();
    assert_eq!(
        exes,
        vec!["/bin/second"],
        "the first group is not re-read after a reconnect"
    );
}

#[test]
fn incomplete_groups_never_emit_and_are_bounded() {
    let mut text = String::new();
    for id in 0..(AUDIT_PENDING_CAP as u32 + 50) {
        text.push_str(&format!("type=CWD msg=audit(1.0:{id}): cwd=\"/\"\n"));
    }
    text.push_str("type=PROCTITLE msg=audit(1.0:7): proctitle=\"/bin/x\"\n");
    let mut reader = Cursor::new(text);
    let mut pending = HashMap::new();
    let mut emitted = 0;
    tail_audit_log(&mut reader, &mut pending, &mut |_| {
        emitted += 1;
        true
    })
    .unwrap();
    assert_eq!(emitted, 0, "a PROCTITLE without a SYSCALL is not a record");
    assert!(
        pending.len() <= AUDIT_PENDING_CAP,
        "pending must be bounded, has {}",
        pending.len()
    );
}

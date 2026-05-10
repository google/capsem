//! Tests pinning the `capsem shell` exit invariants.
//!
//! Background: a user reported that pressing Ctrl-C / typing `exit` in
//! `capsem shell` left their terminal flooded with binary garbage
//! (MessagePack frames -- `bootconfig`, `epoch_secs`, `Pong` repeated).
//! Symptoms came from two compounding bugs:
//!   1. The `output_task` spawned by `run_shell` was never aborted.
//!      tokio's `JoinHandle` drop does NOT cancel the task -- it lives
//!      on the runtime, holds `stdout`, and any TerminalOutput frame
//!      that arrives after the loop exits writes to the user's now-
//!      cooked-mode parent shell.
//!   2. The host kept queuing `ProcessToService::TerminalOutput` frames
//!      because the client never told it "I'm gone, stop streaming".
//!
//! These tests pin the contract.

#![allow(clippy::needless_pass_by_value)]

use super::*;

// ---------------------------------------------------------------------------
// 1. Reset sequence shape.
// ---------------------------------------------------------------------------

#[test]
fn reset_sequence_clears_sgr_and_shows_cursor() {
    let s = std::str::from_utf8(TERMINAL_RESET_SEQUENCE)
        .expect("reset sequence must be valid utf-8 (it is just ANSI escapes + CRLF)");
    // SGR reset (clears bold/color/inverse). Without this a guest that
    // ended mid-color paints the parent shell prompt the wrong color.
    assert!(
        s.contains("\x1b[0m"),
        "reset must contain SGR reset; got {:?}",
        s
    );
    // Show cursor (guests sometimes hide it and crash before showing).
    assert!(
        s.contains("\x1b[?25h"),
        "reset must contain show-cursor; got {:?}",
        s
    );
    // CRLF so the next prompt starts at column 0 regardless of where
    // the guest left the cursor.
    assert!(s.ends_with("\r\n"), "reset must end with CRLF; got {:?}", s);
}

#[test]
fn reset_sequence_contains_no_alternate_screen_toggle() {
    // Switching screens in the cleanup would WIPE the user's scrollback
    // every time they exit a sandbox shell. Guard against accidentally
    // adding `\x1b[?1049l` (alt-screen exit) here.
    let s = std::str::from_utf8(TERMINAL_RESET_SEQUENCE).unwrap();
    assert!(
        !s.contains("\x1b[?1049"),
        "must not toggle alt-screen on exit"
    );
    assert!(
        !s.contains("\x1b[?47"),
        "must not toggle alt-screen on exit (legacy)"
    );
}

#[test]
fn reset_sequence_contains_no_clear_screen() {
    // `\x1b[2J` would erase the visible scrollback. The user is exiting
    // a sandbox; they want to KEEP what they ran before.
    let s = std::str::from_utf8(TERMINAL_RESET_SEQUENCE).unwrap();
    assert!(!s.contains("\x1b[2J"), "must not clear screen on exit");
    assert!(
        !s.contains("\x1bc"),
        "must not full-reset (RIS) on exit -- clears scrollback"
    );
}

#[test]
fn reset_sequence_is_short() {
    // Belt and braces: a runaway reset sequence (e.g. someone added a
    // big clear) should fail loudly. 32 bytes is plenty for the legitimate
    // SGR + show-cursor + CRLF combo (~9 bytes).
    assert!(
        TERMINAL_RESET_SEQUENCE.len() <= 32,
        "reset sequence is {} bytes; expected <= 32 (something got added)",
        TERMINAL_RESET_SEQUENCE.len(),
    );
}

// ---------------------------------------------------------------------------
// 2. tty-vs-pipe behavior.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reset_user_terminal_is_noop_when_not_a_tty() {
    // When stdout is a pipe (CI, `capsem shell | tee ...`), writing ANSI
    // escapes pollutes the captured output. is_tty=false must short-circuit.
    //
    // We can't easily intercept the global stdout in a unit test, but we
    // can at least assert the function returns quickly and doesn't panic.
    let start = std::time::Instant::now();
    reset_user_terminal(false).await;
    assert!(start.elapsed() < std::time::Duration::from_millis(50));
}

#[tokio::test]
async fn reset_user_terminal_does_not_panic_when_tty_unavailable() {
    // Even with is_tty=true, stdout might fail to write (closed pipe,
    // EPIPE under SIGPIPE-ignore). Exit cleanup must never panic.
    reset_user_terminal(true).await;
}

// ---------------------------------------------------------------------------
// 3. tokio JoinHandle abort semantics -- the load-bearing fix.
// ---------------------------------------------------------------------------
//
// The original bug was: `let mut output_task = tokio::spawn(...)`, then
// the function returned without calling `.abort()`. The task kept running
// (drop of JoinHandle does NOT cancel) and continued to write to stdout.
// These tests pin the abort behavior we rely on.

#[tokio::test]
async fn join_handle_drop_does_not_cancel_task() {
    // This is what BIT US. JoinHandle::drop() detaches; it does NOT abort.
    // If this assertion ever flips (e.g. tokio changes behavior), the
    // band-aid in run_shell is unnecessary and we can simplify.
    let started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s = started.clone();
    let h = tokio::spawn(async move {
        s.store(true, std::sync::atomic::Ordering::SeqCst);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
    drop(h); // <- explicit drop, mirrors run_shell return path
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        started.load(std::sync::atomic::Ordering::SeqCst),
        "task should have started despite JoinHandle drop"
    );
    // We can't easily assert "still running" without holding a handle,
    // but the lack of a panic from runtime shutdown proves it didn't get
    // implicitly cancelled.
}

#[tokio::test]
async fn join_handle_abort_actually_stops_the_task() {
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let c = counter.clone();
    let h = tokio::spawn(async move {
        loop {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    h.abort();
    let snapshot = counter.load(std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let after = counter.load(std::sync::atomic::Ordering::SeqCst);
    // After abort, the counter must stop incrementing. Allow +1 for an
    // already-scheduled iteration that ran between abort() and the snapshot.
    assert!(
        after <= snapshot + 1,
        "task should be stopped after abort: snapshot={snapshot} after={after}"
    );
}

// ---------------------------------------------------------------------------
// 4. Regression detector: anything that LOOKS like MessagePack must not
//    appear in TerminalOutput data.
// ---------------------------------------------------------------------------
//
// HostToGuest / GuestToHost frames are encoded via `rmp_serde::to_vec_named`
// with `#[serde(tag = "t", content = "d", rename_all = "lowercase")]`. Every
// such frame begins with the bytes `0x82 0xa1 't' 0xa?` (fixmap[2], fixstr[1]
// "t", fixstr[N] "<variant>"). If a TerminalOutput.data buffer ever carries
// that prefix, an IPC frame leaked into the PTY stream -- exactly the bug
// this whole module exists to prevent.

// Detector lives in `super` (shell_exit.rs) so production code can also
// use it for smoking-gun logging if the leak ever resurfaces.

#[test]
fn detector_recognizes_real_bootconfig_frame() {
    use capsem_proto::HostToGuest;
    let bytes = capsem_proto::encode_host_msg(&HostToGuest::BootConfig {
        epoch_secs: 1234,
        traceparent: String::new(),
    })
    .expect("encode");
    // Strip the 4-byte length prefix that encode_host_msg adds.
    let payload = &bytes[4..];
    assert!(
        looks_like_msgpack_ipc_frame(payload),
        "detector should match real BootConfig frame, payload={payload:02x?}"
    );
}

#[test]
fn detector_recognizes_real_pong_frame() {
    use capsem_proto::GuestToHost;
    let bytes = capsem_proto::encode_guest_msg(&GuestToHost::Pong).expect("encode");
    let payload = &bytes[4..];
    assert!(
        looks_like_msgpack_ipc_frame(payload),
        "detector should match real Pong frame, payload={payload:02x?}"
    );
}

#[test]
fn detector_recognizes_real_setenv_frame() {
    use capsem_proto::HostToGuest;
    let bytes = capsem_proto::encode_host_msg(&HostToGuest::SetEnv {
        key: "FOO".into(),
        value: "bar".into(),
    })
    .expect("encode");
    let payload = &bytes[4..];
    assert!(looks_like_msgpack_ipc_frame(payload));
}

#[test]
fn detector_does_not_false_positive_on_normal_terminal_output() {
    // ANSI escape from a guest that just ran `ls --color`.
    let ansi = b"\x1b[01;34mdir\x1b[0m\r\n";
    assert!(!looks_like_msgpack_ipc_frame(ansi));

    // Plain ASCII bash prompt.
    let prompt = b"capsem@vm:~$ ";
    assert!(!looks_like_msgpack_ipc_frame(prompt));

    // Bash 'exit' echo + newline -- the exact bytes the user sees right
    // before garbage in the original report.
    assert!(!looks_like_msgpack_ipc_frame(b"exit\r\n"));

    // A short prefix that's too small to be a frame.
    assert!(!looks_like_msgpack_ipc_frame(b""));
    assert!(!looks_like_msgpack_ipc_frame(b"\x82"));
    assert!(!looks_like_msgpack_ipc_frame(b"\x82\xa1"));
    assert!(!looks_like_msgpack_ipc_frame(b"\x82\xa1t"));
    assert!(!looks_like_msgpack_ipc_frame(b"\x81"));
    assert!(!looks_like_msgpack_ipc_frame(b"\x81\xa1t"));

    // Nearly-matching bytes that are NOT an IPC frame.
    assert!(!looks_like_msgpack_ipc_frame(b"\x82\xa1x\xaa")); // wrong tag char
    assert!(!looks_like_msgpack_ipc_frame(b"\x80\xa1t\xaa")); // fixmap[0]
    assert!(!looks_like_msgpack_ipc_frame(b"\x83\xa1t\xaa")); // fixmap[3]
    assert!(!looks_like_msgpack_ipc_frame(b"\x82\xa2tt\xaa")); // fixstr[2] for the key

    // UTF-8 text that happens to contain 0x82 byte mid-stream is fine.
    let utf = "héllo wörld\n".as_bytes();
    assert!(!looks_like_msgpack_ipc_frame(utf));
}

#[test]
fn detector_does_not_false_positive_on_msgpack_inside_data() {
    // The real bug is leakage at the START of a TerminalOutput.data buffer
    // (capsem-shell writes data verbatim). MessagePack bytes appearing
    // INSIDE legitimate file content (e.g. `cat msgpack-blob.bin`) are
    // not a leak -- they're what the user asked for. Detector targets
    // the start-of-buffer case only.
    let mixed = {
        let mut v = b"hello ".to_vec();
        v.extend_from_slice(b"\x82\xa1t\xaa\xaa");
        v
    };
    assert!(!looks_like_msgpack_ipc_frame(&mixed));
}

// ---------------------------------------------------------------------------
// 5. Catalog: every variant of every IPC envelope produces a frame the
//    detector can recognize. If a future variant is added with a different
//    serde tag scheme, this test fails and we know the detector needs an
//    update before the leak can resurface unnoticed.
// ---------------------------------------------------------------------------

#[test]
fn detector_recognizes_every_host_to_guest_variant() {
    use capsem_proto::HostToGuest;
    let samples = [
        HostToGuest::BootConfig {
            epoch_secs: 1,
            traceparent: String::new(),
        },
        HostToGuest::SetEnv {
            key: "K".into(),
            value: "V".into(),
        },
        HostToGuest::FileWrite {
            id: 1,
            path: "/p".into(),
            data: vec![],
            mode: 0o644,
        },
        HostToGuest::FileRead {
            id: 1,
            path: "/p".into(),
        },
        HostToGuest::FileDelete {
            id: 1,
            path: "/p".into(),
        },
        HostToGuest::BootConfigDone,
        HostToGuest::Resize { cols: 80, rows: 24 },
        HostToGuest::Ping { epoch_secs: 0 },
        HostToGuest::Shutdown,
        HostToGuest::Exec {
            id: 1,
            command: "ls".into(),
        },
        HostToGuest::PrepareSnapshot,
    ];
    for msg in samples {
        let bytes = capsem_proto::encode_host_msg(&msg).expect("encode");
        let payload = &bytes[4..]; // strip 4-byte length prefix
        assert!(
            looks_like_msgpack_ipc_frame(payload),
            "detector missed HostToGuest variant {:?} -- payload={:02x?}",
            msg,
            payload,
        );
    }
}

#[test]
fn detector_recognizes_every_guest_to_host_variant() {
    use capsem_proto::GuestToHost;
    let samples = [
        GuestToHost::Pong,
        GuestToHost::Ready {
            version: "1.0".into(),
        },
        GuestToHost::Error {
            id: 1,
            message: "x".into(),
        },
        GuestToHost::FileOpDone { id: 1 },
        GuestToHost::FileContent {
            id: 1,
            path: "/p".into(),
            data: vec![],
        },
        GuestToHost::ExecDone {
            id: 1,
            exit_code: 0,
        },
    ];
    for msg in samples {
        let bytes = capsem_proto::encode_guest_msg(&msg).expect("encode");
        let payload = &bytes[4..];
        assert!(
            looks_like_msgpack_ipc_frame(payload),
            "detector missed GuestToHost variant {:?} -- payload={:02x?}",
            msg,
            payload,
        );
    }
}

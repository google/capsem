use super::vsock_io::{SockaddrVm, AF_VSOCK};
use super::*;
use std::io::Write;
use std::os::unix::io::{FromRawFd, IntoRawFd};

use super::audit::{extract_audit_id, extract_audit_timestamp_us, extract_execve_argv, extract_field};

mod control_loop;

pub(crate) fn test_ctrl_channel() -> (CtrlSender, SharedCtrlReceiver) {
    CtrlSender::new(std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::new(),
    )))
}

/// Use the standard library's portable close-on-exec pipe construction so
/// children launched by other tests cannot keep these endpoints alive.
fn make_pipe() -> (RawFd, RawFd) {
    let (reader, writer) = std::io::pipe().unwrap();
    (reader.into_raw_fd(), writer.into_raw_fd())
}

// Wire format compatibility: new disjoint types over pipes
// -----------------------------------------------------------------------

#[test]
fn agent_ready_roundtrip() {
    let (read_fd, write_fd) = make_pipe();
    let msg = GuestToHost::Ready {
        version: "0.3.0".to_string(),
    };
    send_guest_msg(write_fd, &msg).unwrap();
    // Simulate host-side receive.
    let mut len_buf = [0u8; 4];
    read_exact_fd(read_fd, &mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    read_exact_fd(read_fd, &mut payload).unwrap();
    let decoded: GuestToHost = capsem_proto::decode_guest_msg(&payload).unwrap();
    match decoded {
        GuestToHost::Ready { version } => assert_eq!(version, "0.3.0"),
        other => panic!("expected Ready, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn host_resize_decodable_by_agent() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::Resize { cols: 200, rows: 50 };
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    match decoded {
        HostToGuest::Resize { cols, rows } => {
            assert_eq!(cols, 200);
            assert_eq!(rows, 50);
        }
        other => panic!("expected Resize, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn boot_config_roundtrip_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::BootConfig {
        epoch_secs: 1708800000,
        traceparent: String::new(),
    };
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    match decoded {
        HostToGuest::BootConfig {
            epoch_secs,
            traceparent: _,
        } => {
            assert_eq!(epoch_secs, 1708800000);
        }
        other => panic!("expected BootConfig, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn boot_handshake_set_env_roundtrip() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::SetEnv {
        key: "TERM".into(),
        value: "xterm-256color".into(),
    };
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    match decoded {
        HostToGuest::SetEnv { key, value } => {
            assert_eq!(key, "TERM");
            assert_eq!(value, "xterm-256color");
        }
        other => panic!("expected SetEnv, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn boot_handshake_file_write_roundtrip() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::FileWrite {
        id: 1,
        path: "/root/.gemini/settings.json".into(),
        data: b"{}".to_vec(),
        mode: 0o644,
    };
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    match decoded {
        HostToGuest::FileWrite { id, path, data, mode } => {
            assert_eq!(id, 1);
            assert_eq!(path, "/root/.gemini/settings.json");
            assert_eq!(data, b"{}");
            assert_eq!(mode, 0o644);
        }
        other => panic!("expected FileWrite, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn boot_config_done_roundtrip() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::BootConfigDone;
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    assert!(matches!(decoded, HostToGuest::BootConfigDone));
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn boot_ready_roundtrip_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    send_guest_msg(write_fd, &GuestToHost::BootReady).unwrap();
    let mut len_buf = [0u8; 4];
    read_exact_fd(read_fd, &mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    read_exact_fd(read_fd, &mut payload).unwrap();
    let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
    assert!(matches!(decoded, GuestToHost::BootReady));
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn send_recv_exec_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::Exec {
        id: 99,
        command: "echo hi".to_string(),
    };
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    match decoded {
        HostToGuest::Exec { id, command } => {
            assert_eq!(id, 99);
            assert_eq!(command, "echo hi");
        }
        other => panic!("expected Exec, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn send_recv_exec_done_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    send_guest_msg(write_fd, &GuestToHost::ExecDone { id: 99, exit_code: 1 }).unwrap();
    let mut len_buf = [0u8; 4];
    read_exact_fd(read_fd, &mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    read_exact_fd(read_fd, &mut payload).unwrap();
    let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
    match decoded {
        GuestToHost::ExecDone { id, exit_code } => {
            assert_eq!(id, 99);
            assert_eq!(exit_code, 1);
        }
        other => panic!("expected ExecDone, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn prepare_snapshot_roundtrip() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::PrepareSnapshot;
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    assert!(matches!(decoded, HostToGuest::PrepareSnapshot));
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn unfreeze_roundtrip() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::Unfreeze;
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    assert!(matches!(decoded, HostToGuest::Unfreeze));
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn snapshot_ready_roundtrip() {
    let (read_fd, write_fd) = make_pipe();
    send_guest_msg(write_fd, &GuestToHost::SnapshotReady).unwrap();
    let mut len_buf = [0u8; 4];
    read_exact_fd(read_fd, &mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    read_exact_fd(read_fd, &mut payload).unwrap();
    let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
    assert!(matches!(decoded, GuestToHost::SnapshotReady));
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn send_recv_multiple_messages_over_pipe() {
    let (read_fd, write_fd) = make_pipe();

    // Send host messages.
    let ping_frame = capsem_proto::encode_host_msg(&HostToGuest::Ping { epoch_secs: 0 }).unwrap();
    write_all_fd(write_fd, &ping_frame).unwrap();
    let resize_frame = capsem_proto::encode_host_msg(&HostToGuest::Resize { cols: 80, rows: 24 }).unwrap();
    write_all_fd(write_fd, &resize_frame).unwrap();

    assert!(matches!(recv_host_msg(read_fd).unwrap(), HostToGuest::Ping { .. }));
    match recv_host_msg(read_fd).unwrap() {
        HostToGuest::Resize { cols, rows } => {
            assert_eq!(cols, 80);
            assert_eq!(rows, 24);
        }
        other => panic!("expected Resize, got {other:?}"),
    }

    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn recv_rejects_oversized_frame() {
    let (read_fd, write_fd) = make_pipe();
    // Write a length prefix claiming > MAX_FRAME_SIZE.
    let len_bytes = (MAX_FRAME_SIZE + 1).to_be_bytes();
    let mut writer = unsafe { std::fs::File::from_raw_fd(write_fd) };
    writer.write_all(&len_bytes).unwrap();
    std::mem::forget(writer);

    let result = recv_host_msg(read_fd);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);

    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn recv_eof_returns_error() {
    let (read_fd, write_fd) = make_pipe();
    unsafe {
        libc::close(write_fd);
    }
    let result = recv_host_msg(read_fd);
    assert!(result.is_err());
    unsafe {
        libc::close(read_fd);
    }
}

// -----------------------------------------------------------------------
// Clock sync
// -----------------------------------------------------------------------

#[test]
fn set_system_clock_no_crash() {
    // On non-root systems this will fail with EPERM, but must not crash.
    set_system_clock(1708800000);
}

// -----------------------------------------------------------------------
// SockaddrVm struct layout
// -----------------------------------------------------------------------

#[test]
fn sockaddr_vm_size_matches_kernel() {
    assert_eq!(
        std::mem::size_of::<SockaddrVm>(),
        16,
        "SockaddrVm must be 16 bytes to match kernel struct"
    );
}

#[test]
fn sockaddr_vm_field_offsets() {
    let addr = SockaddrVm {
        svm_family: 0,
        svm_reserved1: 0,
        svm_port: 0,
        svm_cid: 0,
        svm_flags: 0,
        svm_zero: [0; 3],
    };
    let base = &addr as *const _ as usize;
    let family_offset = &addr.svm_family as *const _ as usize - base;
    let port_offset = &addr.svm_port as *const _ as usize - base;
    let cid_offset = &addr.svm_cid as *const _ as usize - base;
    assert_eq!(family_offset, 0, "svm_family must be at offset 0");
    assert_eq!(port_offset, 4, "svm_port must be at offset 4");
    assert_eq!(cid_offset, 8, "svm_cid must be at offset 8");
}

// -----------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------

#[test]
fn port_constants_match_host() {
    assert_eq!(VSOCK_PORT_CONTROL, 5000);
    assert_eq!(VSOCK_PORT_TERMINAL, 5001);
}

#[test]
fn host_cid_is_two() {
    assert_eq!(VSOCK_HOST_CID, 2);
}

#[test]
fn af_vsock_is_40() {
    assert_eq!(AF_VSOCK, 40);
}

// -----------------------------------------------------------------------
// PTY winsize
// -----------------------------------------------------------------------

#[test]
fn set_winsize_on_real_pty() {
    let pty = openpty(None, None).expect("openpty failed");
    let master_fd = pty.master.as_raw_fd();
    set_winsize(master_fd, 200, 50);

    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::ioctl(master_fd, libc::TIOCGWINSZ, &mut ws) };
    assert_eq!(ret, 0);
    assert_eq!(ws.ws_col, 200);
    assert_eq!(ws.ws_row, 50);
}

#[test]
fn set_winsize_boundary_values() {
    let pty = openpty(None, None).expect("openpty failed");
    let master_fd = pty.master.as_raw_fd();

    set_winsize(master_fd, 1, 1);
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    unsafe {
        libc::ioctl(master_fd, libc::TIOCGWINSZ, &mut ws);
    }
    assert_eq!(ws.ws_col, 1);
    assert_eq!(ws.ws_row, 1);

    set_winsize(master_fd, 500, 200);
    unsafe {
        libc::ioctl(master_fd, libc::TIOCGWINSZ, &mut ws);
    }
    assert_eq!(ws.ws_col, 500);
    assert_eq!(ws.ws_row, 200);
}

// -----------------------------------------------------------------------
// Bridge loop concurrency
// -----------------------------------------------------------------------

#[test]
fn bridge_loop_transfers_multi_chunk_data_both_directions() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (mut master_host, master_guest) = UnixStream::pair().unwrap();
    let (mut vsock_host, vsock_guest) = UnixStream::pair().unwrap();

    let timeout = std::time::Duration::from_secs(30);
    master_host.set_read_timeout(Some(timeout)).unwrap();
    master_host.set_write_timeout(Some(timeout)).unwrap();
    vsock_host.set_read_timeout(Some(timeout)).unwrap();
    vsock_host.set_write_timeout(Some(timeout)).unwrap();

    let master_fd = master_guest.into_raw_fd();
    let vsock_fd = vsock_guest.into_raw_fd();

    let _bridge_thread = std::thread::spawn(move || {
        bridge_loop(master_fd, vsock_fd, &HostShutdown::default());
    });

    // 16 KiB is twice the bridge copy buffer, so this still forces
    // multi-chunk full-duplex transfer without turning the unit test into a
    // throughput benchmark under llvm-cov.
    let data_size = 16 * 1024;
    let test_data = vec![0x42u8; data_size];

    let mut master_host_read = master_host.try_clone().unwrap();
    let mut vsock_host_read = vsock_host.try_clone().unwrap();
    master_host_read.set_read_timeout(Some(timeout)).unwrap();
    vsock_host_read.set_read_timeout(Some(timeout)).unwrap();

    std::io::Write::write_all(&mut master_host, &test_data).unwrap();
    let mut vsock_out = vec![0u8; data_size];
    std::io::Read::read_exact(&mut vsock_host_read, &mut vsock_out).unwrap();
    assert_eq!(vsock_out, test_data);

    std::io::Write::write_all(&mut vsock_host, &test_data).unwrap();
    let mut master_out = vec![0u8; data_size];
    std::io::Read::read_exact(&mut master_host_read, &mut master_out).unwrap();
    assert_eq!(master_out, test_data);
}

#[test]
fn bridge_loop_shuts_down_vsock_before_returning() {
    use std::os::unix::io::IntoRawFd;
    use std::os::unix::net::UnixStream;

    let (master_host, master_guest) = UnixStream::pair().unwrap();
    let (mut vsock_host, vsock_guest) = UnixStream::pair().unwrap();
    vsock_host
        .set_read_timeout(Some(std::time::Duration::from_millis(250)))
        .unwrap();

    let master_fd = master_guest.into_raw_fd();
    let vsock_fd = vsock_guest.into_raw_fd();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let bridge = std::thread::spawn(move || {
        bridge_loop(master_fd, vsock_fd, &HostShutdown::default());
        done_tx.send(()).unwrap();
    });

    drop(master_host);
    done_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("bridge loop did not return after the PTY disconnected");

    let mut byte = [0u8; 1];
    let peer_read = std::io::Read::read(&mut vsock_host, &mut byte);

    unsafe {
        libc::shutdown(vsock_fd, libc::SHUT_RDWR);
        libc::close(master_fd);
        libc::close(vsock_fd);
    }
    bridge.join().unwrap();

    assert_eq!(
        peer_read.unwrap(),
        0,
        "bridge_loop returned while its vsock reader was still alive"
    );
}

#[test]
fn boot_log_defaults_to_host_visible_workspace() {
    assert_eq!(BOOT_LOG_PATH, "/root/.capsem-agent-boot.log");
    assert_eq!(FALLBACK_BOOT_LOG_PATH, "/var/log/capsem-boot.log");
}

// -----------------------------------------------------------------------
// Boot timing parser
// -----------------------------------------------------------------------

#[test]
fn parse_boot_timing_valid_jsonl() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing");
    std::fs::write(
        &path,
        "{\"name\":\"squashfs\",\"duration_ms\":50}\n{\"name\":\"network\",\"duration_ms\":120}\n",
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].name, "squashfs");
    assert_eq!(result[0].duration_ms, 50);
    assert_eq!(result[1].name, "network");
    assert_eq!(result[1].duration_ms, 120);
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_missing_file() {
    let result = parse_boot_timing("/nonexistent/capsem-boot-timing");
    assert!(result.is_empty());
}

#[test]
fn parse_boot_timing_skips_malformed_lines() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-bad");
    std::fs::write(
        &path,
        "{\"name\":\"good\",\"duration_ms\":100}\nnot json\n{\"name\":\"also_good\",\"duration_ms\":200}\n",
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].name, "good");
    assert_eq!(result[1].name, "also_good");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_rejects_xss_names() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-xss");
    std::fs::write(
        &path,
        concat!(
            "{\"name\":\"<script>alert(1)</script>\",\"duration_ms\":10}\n",
            "{\"name\":\"normal\",\"duration_ms\":20}\n",
            "{\"name\":\"a]};fetch('http://evil')\",\"duration_ms\":30}\n",
            "{\"name\":\"\",\"duration_ms\":40}\n",
            "{\"name\":\"has spaces\",\"duration_ms\":50}\n",
            "{\"name\":\"path/../traversal\",\"duration_ms\":60}\n",
        ),
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1, "only 'normal' should survive: {result:?}");
    assert_eq!(result[0].name, "normal");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_rejects_huge_duration() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-huge");
    std::fs::write(
        &path,
        concat!(
            "{\"name\":\"ok\",\"duration_ms\":1000}\n",
            "{\"name\":\"huge\",\"duration_ms\":999999999}\n",
        ),
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "ok");
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_caps_at_32_entries() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-cap");
    let mut lines = String::new();
    for i in 0..50 {
        use std::fmt::Write as _;
        writeln!(lines, "{{\"name\":\"stage{i}\",\"duration_ms\":{i}}}").unwrap();
    }
    std::fs::write(&path, &lines).unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 32);
    std::fs::remove_file(&path).ok();
}

// -------------------------------------------------------------------
// O_NOFOLLOW file I/O helpers
// -------------------------------------------------------------------

#[test]
fn write_nofollow_works_for_regular_file() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-write-nofollow");
    write_nofollow(path.to_str().unwrap(), b"hello", 0o644).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    std::fs::remove_file(&path).ok();
}

#[test]
fn read_nofollow_works_for_regular_file() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-read-nofollow");
    std::fs::write(&path, b"world").unwrap();
    assert_eq!(read_nofollow(path.to_str().unwrap(), usize::MAX).unwrap(), b"world");
    std::fs::remove_file(&path).ok();
}

#[test]
fn read_nofollow_refuses_a_file_over_the_frame_budget() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-read-nofollow-budget");
    std::fs::write(&path, b"0123456789").unwrap();
    let err = read_nofollow(path.to_str().unwrap(), 9).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::FileTooLarge, "{err}");
    assert_eq!(read_nofollow(path.to_str().unwrap(), 10).unwrap(), b"0123456789");
    std::fs::remove_file(&path).ok();
}

#[test]
fn file_read_reply_for_an_oversized_file_is_an_error_that_fits_a_frame() {
    // A reply the host cannot decode is never acked and replays forever, so
    // the reply for a file that does not fit must be a small Error frame.
    let path = "/root/blob.bin".to_string();
    let too_big = GuestToHost::FileContent {
        id: 7,
        path: path.clone(),
        data: vec![0xFF; MAX_FRAME_SIZE as usize],
    };
    assert!(!capsem_proto::guest_msg_fits_frame(&too_big));
    let reply = GuestToHost::Error {
        id: 7,
        message: format!("failed to read {path}: file too large for one control frame ({MAX_FRAME_SIZE} bytes)"),
    };
    assert!(capsem_proto::guest_msg_fits_frame(&reply));
}

#[test]
fn frame_or_drop_evicts_an_unframeable_reply_from_the_replay_map() {
    let pending = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let too_big = GuestToHost::FileContent {
        id: 7,
        path: "/root/blob.bin".to_string(),
        data: vec![0xFF; MAX_FRAME_SIZE as usize],
    };
    pending.lock().unwrap().insert(7, too_big.clone());
    assert!(frame_or_drop(&too_big, &pending).is_none());
    assert!(
        pending.lock().unwrap().is_empty(),
        "a frame the host would never ack must not be replayed on every reconnect"
    );
    let ack = GuestToHost::Ack { id: 7 };
    assert!(frame_or_drop(&ack, &pending).is_some());
}

#[test]
fn write_nofollow_rejects_symlink() {
    let dir = std::env::temp_dir();
    let target = dir.join("capsem-test-wn-target");
    let link = dir.join("capsem-test-wn-link");
    std::fs::write(&target, b"original").unwrap();
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let err = write_nofollow(link.to_str().unwrap(), b"evil", 0o644);
    assert!(err.is_err(), "write through symlink must fail");
    // Target must be unchanged.
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    std::fs::remove_file(&target).ok();
    std::fs::remove_file(&link).ok();
}

#[test]
fn read_nofollow_rejects_symlink() {
    let dir = std::env::temp_dir();
    let target = dir.join("capsem-test-rn-target");
    let link = dir.join("capsem-test-rn-link");
    std::fs::write(&target, b"secret").unwrap();
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let err = read_nofollow(link.to_str().unwrap(), usize::MAX);
    assert!(err.is_err(), "read through symlink must fail");
    std::fs::remove_file(&target).ok();
    std::fs::remove_file(&link).ok();
}

#[test]
fn delete_nofollow_rejects_symlink() {
    let dir = std::env::temp_dir();
    let target = dir.join("capsem-test-dn-target");
    let link = dir.join("capsem-test-dn-link");
    std::fs::write(&target, b"keep").unwrap();
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let err = delete_nofollow(link.to_str().unwrap());
    assert!(err.is_err(), "delete of symlink must fail");
    // Both should still exist.
    assert!(target.exists());
    assert!(link.symlink_metadata().is_ok());
    std::fs::remove_file(&target).ok();
    std::fs::remove_file(&link).ok();
}

#[test]
fn delete_nofollow_deletes_regular_file() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-dn-regular");
    std::fs::write(&path, b"delete me").unwrap();
    delete_nofollow(path.to_str().unwrap()).unwrap();
    assert!(!path.exists());
}

#[test]
fn delete_nofollow_nonexistent_returns_error() {
    let result = delete_nofollow("/tmp/capsem-test-dn-nonexistent-xyzzy");
    assert!(result.is_err());
}

#[test]
fn write_nofollow_creates_parent_dirs() {
    let dir = std::env::temp_dir();
    let nested = dir.join("capsem-test-wn-nested/deep/path/file.txt");
    let _ = std::fs::remove_dir_all(dir.join("capsem-test-wn-nested"));
    write_nofollow(nested.to_str().unwrap(), b"nested", 0o644).unwrap();
    assert_eq!(std::fs::read(&nested).unwrap(), b"nested");
    std::fs::remove_dir_all(dir.join("capsem-test-wn-nested")).ok();
}

#[test]
fn write_nofollow_sets_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-wn-perms");
    write_nofollow(path.to_str().unwrap(), b"test", 0o755).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
    std::fs::remove_file(&path).ok();
}

#[test]
fn write_nofollow_truncates_existing() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-wn-truncate");
    write_nofollow(path.to_str().unwrap(), b"long content here", 0o644).unwrap();
    write_nofollow(path.to_str().unwrap(), b"short", 0o644).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"short");
    std::fs::remove_file(&path).ok();
}

#[test]
fn read_nofollow_nonexistent_returns_error() {
    let result = read_nofollow("/tmp/capsem-test-rn-nonexistent-xyzzy", usize::MAX);
    assert!(result.is_err());
}

// -------------------------------------------------------------------
// Boot timing: additional edge cases
// -------------------------------------------------------------------

#[test]
fn parse_boot_timing_empty_file() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-empty");
    std::fs::write(&path, "").unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert!(result.is_empty());
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_rejects_long_names() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-longname");
    let long_name = "a".repeat(65);
    std::fs::write(
        &path,
        format!(
            "{{\"name\":\"{long_name}\",\"duration_ms\":10}}\n\
             {{\"name\":\"ok\",\"duration_ms\":20}}\n"
        ),
    )
    .unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "ok");
    std::fs::remove_file(&path).ok();
}

// -------------------------------------------------------------------
// Control message: frame boundary
// -------------------------------------------------------------------

#[test]
fn recv_truncated_payload_returns_error() {
    let (read_fd, write_fd) = make_pipe();
    // Write a valid length (10 bytes) but only 5 bytes of payload
    let len_bytes = 10u32.to_be_bytes();
    write_all_fd(write_fd, &len_bytes).unwrap();
    write_all_fd(write_fd, &[0u8; 5]).unwrap();
    unsafe {
        libc::close(write_fd);
    }

    let result = recv_host_msg(read_fd);
    assert!(result.is_err());
    unsafe {
        libc::close(read_fd);
    }
}

#[test]
fn send_recv_pong_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    send_guest_msg(write_fd, &GuestToHost::Pong).unwrap();
    let mut len_buf = [0u8; 4];
    read_exact_fd(read_fd, &mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    read_exact_fd(read_fd, &mut payload).unwrap();
    let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
    assert!(matches!(decoded, GuestToHost::Pong));
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn send_recv_file_content_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    let msg = GuestToHost::FileContent {
        id: 42,
        path: "/root/test.txt".to_string(),
        data: b"file contents here".to_vec(),
    };
    send_guest_msg(write_fd, &msg).unwrap();
    let mut len_buf = [0u8; 4];
    read_exact_fd(read_fd, &mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    read_exact_fd(read_fd, &mut payload).unwrap();
    let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
    match decoded {
        GuestToHost::FileContent { id, path, data } => {
            assert_eq!(id, 42);
            assert_eq!(path, "/root/test.txt");
            assert_eq!(data, b"file contents here");
        }
        other => panic!("expected FileContent, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn send_recv_error_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    let msg = GuestToHost::Error {
        id: 7,
        message: "something went wrong".to_string(),
    };
    send_guest_msg(write_fd, &msg).unwrap();
    let mut len_buf = [0u8; 4];
    read_exact_fd(read_fd, &mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    read_exact_fd(read_fd, &mut payload).unwrap();
    let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
    match decoded {
        GuestToHost::Error { id, message } => {
            assert_eq!(id, 7);
            assert_eq!(message, "something went wrong");
        }
        other => panic!("expected Error, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn send_recv_file_op_done_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    send_guest_msg(write_fd, &GuestToHost::FileOpDone { id: 99 }).unwrap();
    let mut len_buf = [0u8; 4];
    read_exact_fd(read_fd, &mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    read_exact_fd(read_fd, &mut payload).unwrap();
    let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
    match decoded {
        GuestToHost::FileOpDone { id } => assert_eq!(id, 99),
        other => panic!("expected FileOpDone, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

// -------------------------------------------------------------------
// Host message roundtrips: FileRead, FileDelete
// -------------------------------------------------------------------

#[test]
fn file_read_roundtrip_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::FileRead {
        id: 10,
        path: "/root/readme.md".into(),
    };
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    match decoded {
        HostToGuest::FileRead { id, path } => {
            assert_eq!(id, 10);
            assert_eq!(path, "/root/readme.md");
        }
        other => panic!("expected FileRead, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

#[test]
fn file_delete_roundtrip_over_pipe() {
    let (read_fd, write_fd) = make_pipe();
    let msg = HostToGuest::FileDelete {
        id: 11,
        path: "/root/temp.txt".into(),
    };
    let frame = capsem_proto::encode_host_msg(&msg).unwrap();
    write_all_fd(write_fd, &frame).unwrap();
    let decoded = recv_host_msg(read_fd).unwrap();
    match decoded {
        HostToGuest::FileDelete { id, path } => {
            assert_eq!(id, 11);
            assert_eq!(path, "/root/temp.txt");
        }
        other => panic!("expected FileDelete, got {other:?}"),
    }
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }
}

// -------------------------------------------------------------------
// Boot timing: exact boundary
// -------------------------------------------------------------------

#[test]
fn parse_boot_timing_name_at_exact_boundary() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-boundary");
    let name_64 = "a".repeat(64); // exactly at limit, should pass
    std::fs::write(&path, format!("{{\"name\":\"{name_64}\",\"duration_ms\":10}}\n")).unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, name_64);
    std::fs::remove_file(&path).ok();
}

#[test]
fn parse_boot_timing_duration_at_exact_boundary() {
    let dir = std::env::temp_dir();
    let path = dir.join("capsem-test-boot-timing-dur-boundary");
    // 600_000 is exactly at limit, should pass
    std::fs::write(&path, "{\"name\":\"ok\",\"duration_ms\":600000}\n").unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].duration_ms, 600_000);

    // 600_001 is over limit, should be rejected
    std::fs::write(&path, "{\"name\":\"bad\",\"duration_ms\":600001}\n").unwrap();
    let result = parse_boot_timing(path.to_str().unwrap());
    assert!(result.is_empty());
    std::fs::remove_file(&path).ok();
}

// ── auditd record parsing ──────────────────────────────────────────
//
// These four parsers turn raw auditd lines into the exec attribution the
// security ledger records: who ran what, when, under which pid. They are
// string parsers over kernel-formatted text and had no tests. Everything below
// pins observed behaviour, including the sharp edges, so a future rewrite has
// to make a deliberate choice rather than silently changing what gets audited.

const SYSCALL_LINE: &str = concat!(
    "type=SYSCALL msg=audit(1713100000.001:42): arch=c000003e syscall=59 ",
    "success=yes exit=0 ppid=100 pid=200 auid=1000 uid=0 gid=0 ",
    "tty=pts0 comm=\"python3\" exe=\"/usr/bin/python3\" key=\"exec\""
);

#[test]
fn audit_id_is_read_from_the_msg_envelope() {
    assert_eq!(extract_audit_id(SYSCALL_LINE).as_deref(), Some("1713100000.001:42"));
}

#[test]
fn audit_id_is_none_when_the_envelope_is_absent_or_unterminated() {
    for line in [
        "",
        "type=SYSCALL arch=c000003e",               // no msg=audit(
        "type=SYSCALL msg=audit(1713100000.001:42", // no closing paren
    ] {
        assert_eq!(extract_audit_id(line), None, "{line:?}");
    }
}

#[test]
fn audit_timestamp_converts_seconds_to_microseconds() {
    assert_eq!(extract_audit_timestamp_us(SYSCALL_LINE), Some(1_713_100_000_001_000));
}

#[test]
fn audit_timestamp_is_none_when_the_seconds_field_is_not_a_number() {
    for line in ["msg=audit(not-a-number:42):", "msg=audit(:42):", "type=SYSCALL"] {
        assert_eq!(extract_audit_timestamp_us(line), None, "{line:?}");
    }
}

#[test]
fn audit_timestamp_saturates_rather_than_wrapping_on_absurd_input() {
    // Rust float->int casts saturate, so a nonsense timestamp cannot wrap into
    // a plausible-looking value that would silently reorder the ledger.
    assert_eq!(extract_audit_timestamp_us("msg=audit(-1.0:1):"), Some(0));
    assert_eq!(extract_audit_timestamp_us("msg=audit(1e30:1):"), Some(u64::MAX));
}

#[test]
fn field_keys_must_carry_their_leading_space() {
    // extract_field is a substring search, so "pid=" also matches inside
    // "ppid=". Every caller passes " pid=" for exactly this reason. If that
    // space is ever dropped, the ledger silently attributes the parent's pid
    // to the child.
    assert_eq!(extract_field(SYSCALL_LINE, " pid=").as_deref(), Some("200"));
    assert_eq!(extract_field(SYSCALL_LINE, " ppid=").as_deref(), Some("100"));
    assert_eq!(
        extract_field(SYSCALL_LINE, "pid=").as_deref(),
        Some("100"),
        "without the leading space the parent's pid wins -- keep the space"
    );
}

#[test]
fn field_values_stop_at_the_next_space_and_keep_their_quotes() {
    assert_eq!(extract_field(SYSCALL_LINE, " uid=").as_deref(), Some("0"));
    assert_eq!(
        extract_field(SYSCALL_LINE, " comm=").as_deref(),
        Some("\"python3\""),
        "quotes are retained; callers trim them"
    );
}

#[test]
fn field_value_at_end_of_line_needs_no_trailing_space() {
    assert_eq!(extract_field("type=SYSCALL pid=7", " pid=").as_deref(), Some("7"));
}

#[test]
fn field_is_none_for_an_absent_key_or_unterminated_quote() {
    assert_eq!(extract_field(SYSCALL_LINE, " nosuch=").as_deref(), None);
    assert_eq!(
        extract_field("type=SYSCALL comm=\"unterminated", " comm=").as_deref(),
        None,
        "an unclosed quote yields nothing rather than the rest of the line"
    );
}

#[test]
fn execve_argv_is_rebuilt_in_order_and_unquoted() {
    let line = concat!(
        "type=EXECVE msg=audit(1713100000.001:42): argc=3 ",
        "a0=\"python3\" a1=\"train.py\" a2=\"--epochs\""
    );

    assert_eq!(extract_execve_argv(line).as_deref(), Some("python3 train.py --epochs"));
}

#[test]
fn execve_argv_is_none_when_the_record_carries_no_arguments() {
    assert_eq!(
        extract_execve_argv("type=EXECVE msg=audit(1713100000.001:42): argc=0"),
        None
    );
}

#[test]
fn execve_argv_stops_at_the_first_gap_in_the_argument_numbering() {
    // auditd splits over-long arguments into a1_len/a1[0] rather than a plain
    // a1=, which leaves a hole. The scan is sequential, so it stops there and
    // the ledger records a truncated command line rather than a wrong one.
    let line = concat!(
        "type=EXECVE msg=audit(1713100000.001:42): argc=3 ",
        "a0=\"python3\" a2=\"--epochs\""
    );

    assert_eq!(
        extract_execve_argv(line).as_deref(),
        Some("python3"),
        "a gap truncates argv; it must not skip ahead and mis-order arguments"
    );
}

#[test]
fn execve_argv_passes_hex_encoded_arguments_through_unchanged() {
    // auditd hex-encodes any argument containing a space or special character.
    // Nothing decodes it today, so the ledger stores the hex form; asserting it
    // keeps the behaviour visible instead of surprising.
    let line = "type=EXECVE msg=audit(1713100000.001:42): argc=2 a0=\"sh\" a1=2D6C61";

    assert_eq!(extract_execve_argv(line).as_deref(), Some("sh 2D6C61"));
}

// -----------------------------------------------------------------------
// Boot log env preview
// -----------------------------------------------------------------------

#[test]
fn env_preview_truncates_on_a_character_boundary() {
    let straddling = format!("{}é-rest", "a".repeat(39));
    assert_eq!(env_preview(&straddling), format!("{}é...", "a".repeat(39)));
    let straddling_later = format!("{}日本語", "a".repeat(40));
    assert_eq!(env_preview(&straddling_later), format!("{}...", "a".repeat(40)));
    assert_eq!(env_preview("short"), "short");
    assert_eq!(env_preview(&"x".repeat(40)), "x".repeat(40));
    assert_eq!(env_preview(&"x".repeat(41)), format!("{}...", "x".repeat(40)));
    assert_eq!(env_preview(""), "");
}

// -----------------------------------------------------------------------
// Control writer: one channel across reconnects
// -----------------------------------------------------------------------
//
// The writer thread used to own the only receiver of a per-connection
// channel. An exec still running when the connection died finished into a
// dropped receiver, its ExecDone was never parked (the writer did the
// parking), and the host -- which has no exec watchdog -- waited forever.

fn read_guest_frame(host: &mut std::os::unix::net::UnixStream) -> GuestToHost {
    let mut len_buf = [0u8; 4];
    host.read_exact(&mut len_buf).unwrap();
    let mut payload = vec![0u8; u32::from_be_bytes(len_buf) as usize];
    host.read_exact(&mut payload).unwrap();
    capsem_proto::decode_guest_msg(&payload).unwrap()
}

/// Read frames until `want` accepts one; replayed duplicates are allowed.
fn wait_for_frame(host: &mut std::os::unix::net::UnixStream, want: impl Fn(&GuestToHost) -> bool) -> GuestToHost {
    host.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    loop {
        let msg = read_guest_frame(host);
        if want(&msg) {
            return msg;
        }
    }
}

struct WriterConn {
    host: std::os::unix::net::UnixStream,
    guest_fd: RawFd,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

fn spawn_writer_conn(sender: &CtrlSender, rx: &SharedCtrlReceiver) -> WriterConn {
    use std::os::unix::io::IntoRawFd;
    let (host, guest) = std::os::unix::net::UnixStream::pair().unwrap();
    let guest_fd = guest.into_raw_fd();
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let handle = {
        let rx = std::sync::Arc::clone(rx);
        let sender = sender.clone();
        let alive = std::sync::Arc::clone(&alive);
        std::thread::spawn(move || control_writer_loop(&rx, &sender, guest_fd, -1, &alive))
    };
    WriterConn {
        host,
        guest_fd,
        alive,
        handle,
    }
}

impl WriterConn {
    fn finish(self) {
        self.alive.store(false, std::sync::atomic::Ordering::SeqCst);
        drop(self.host);
        self.handle.join().unwrap();
        unsafe {
            libc::close(self.guest_fd);
        }
    }
}

#[test]
fn exec_done_finished_after_a_reconnect_reaches_the_new_connection() {
    let (sender, rx) = test_ctrl_channel();

    let mut first = spawn_writer_conn(&sender, &rx);
    sender.send(GuestToHost::Ack { id: 1 }).unwrap();
    assert!(matches!(read_guest_frame(&mut first.host), GuestToHost::Ack { id: 1 }));
    // The host goes away and the bridge exits.
    first.finish();

    // The exec that was running finishes now, into the same channel.
    sender.send(GuestToHost::ExecDone { id: 7, exit_code: 3 }).unwrap();

    let mut second = spawn_writer_conn(&sender, &rx);
    let done = wait_for_frame(&mut second.host, |m| matches!(m, GuestToHost::ExecDone { id: 7, .. }));
    assert!(matches!(done, GuestToHost::ExecDone { id: 7, exit_code: 3 }));
    second.finish();
}

#[test]
fn a_message_the_dying_writer_consumed_is_delivered_by_the_next_one() {
    let (sender, rx) = test_ctrl_channel();

    let first = spawn_writer_conn(&sender, &rx);
    // The host dies without anyone telling the writer. Shut the socket down
    // rather than only dropping it: a child another test is spawning at this
    // instant briefly holds a copy of this end, and a plain close would let
    // the writer's next write land in that copy's buffer and succeed.
    first.host.shutdown(std::net::Shutdown::Both).unwrap();
    drop(first.host);
    // The writer consumes this, fails to write it, and must hand it on.
    sender.send(GuestToHost::ExecDone { id: 9, exit_code: 0 }).unwrap();
    first.handle.join().unwrap();
    assert!(
        !first.alive.load(std::sync::atomic::Ordering::SeqCst),
        "a failed write ends the connection"
    );
    unsafe {
        libc::close(first.guest_fd);
    }

    let mut second = spawn_writer_conn(&sender, &rx);
    let done = wait_for_frame(&mut second.host, |m| matches!(m, GuestToHost::ExecDone { id: 9, .. }));
    assert!(matches!(done, GuestToHost::ExecDone { id: 9, exit_code: 0 }));
    second.finish();
}

mod control_replay;

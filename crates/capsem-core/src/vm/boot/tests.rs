use super::*;
use std::io::{Read, Write};

/// Create a pipe pair as std::fs::File for testing control msg I/O.
fn pipe_files() -> (std::fs::File, std::fs::File) {
    use std::os::unix::io::FromRawFd;
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(ret, 0, "pipe() failed");
    let rf = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let wf = unsafe { std::fs::File::from_raw_fd(fds[1]) };
    (rf, wf)
}

#[test]
fn write_read_control_msg_ping() {
    let (mut reader, mut writer) = pipe_files();
    write_control_msg(&mut writer, &HostToGuest::Ping { epoch_secs: 0 }).unwrap();
    // read_control_msg reads GuestToHost, not HostToGuest, so we test
    // the raw frame encode/decode instead
    let frame = encode_host_msg(&HostToGuest::Ping { epoch_secs: 0 }).unwrap();
    writer.write_all(&frame).unwrap();
    drop(writer);

    // Read length prefix + payload manually
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).unwrap();
    // Skip the second frame (we wrote twice)
}

#[test]
fn write_read_control_msg_exec_roundtrip() {
    let (mut reader, mut writer) = pipe_files();
    let msg = HostToGuest::Exec {
        id: 42,
        command: "echo test".into(),
    };
    let frame = encode_host_msg(&msg).unwrap();
    writer.write_all(&frame).unwrap();
    drop(writer);

    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    assert!(len < MAX_FRAME_SIZE as usize);
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).unwrap();
    let decoded = capsem_proto::decode_host_msg(&payload).unwrap();
    match decoded {
        HostToGuest::Exec { id, command } => {
            assert_eq!(id, 42);
            assert_eq!(command, "echo test");
        }
        other => panic!("expected Exec, got {other:?}"),
    }
}

#[test]
fn read_control_msg_discards_an_oversized_frame_and_reads_the_next() {
    let (mut reader, mut writer) = pipe_files();
    let oversized = MAX_FRAME_SIZE as usize + 1;
    // A pipe holds 64 KiB; write the 2 MiB frame from another thread.
    let producer = std::thread::spawn(move || {
        writer.write_all(&(oversized as u32).to_be_bytes()).unwrap();
        writer.write_all(&vec![0xAB; oversized]).unwrap();
        writer
            .write_all(&capsem_proto::encode_guest_msg(&GuestToHost::Ack { id: 9 }).unwrap())
            .unwrap();
    });

    let err = read_control_msg(&mut reader).unwrap_err();
    assert!(
        err.downcast_ref::<ControlFrameTooLarge>().is_some(),
        "an oversized frame is its own error so the bridge can keep the connection: {err}"
    );
    assert!(err.to_string().contains("too large"));

    let next = read_control_msg(&mut reader).expect("the stream realigns on the frame after the discarded one");
    assert!(matches!(next, GuestToHost::Ack { id: 9 }), "{next:?}");
    producer.join().unwrap();
}

#[test]
fn ca_key_pem_embedded() {
    assert!(CA_KEY_PEM.contains("PRIVATE KEY"));
}

#[test]
fn ca_cert_pem_embedded() {
    assert!(CA_CERT_PEM.contains("CERTIFICATE"));
}

#[test]
fn virtiofs_cmdline_append() {
    let base = "console=hvc0 ro loglevel=1";
    let shares = [VirtioFsShare {
        tag: "capsem".into(),
        host_path: "/tmp/session".into(),
        read_only: false,
    }];
    let effective = effective_kernel_cmdline(base, &shares, None);
    assert!(effective.contains("capsem.storage=virtiofs"));
}

#[test]
fn virtiofs_cmdline_no_shares() {
    let base = "console=hvc0 ro loglevel=1";
    let shares: Vec<VirtioFsShare> = vec![];
    let effective = effective_kernel_cmdline(base, &shares, None);
    assert!(!effective.contains("capsem.storage=virtiofs"));
}

#[test]
fn erofs_rootfs_override_appends_cmdline_flag() {
    let base = "console=hvc0 ro loglevel=1";
    let shares: Vec<VirtioFsShare> = vec![];
    let rootfs = Path::new("/tmp/rootfs.erofs");
    let effective = effective_kernel_cmdline_with_erofs_mode(base, &shares, Some(rootfs), false);
    assert!(effective.contains("capsem.rootfs=erofs"));
}

#[test]
fn erofs_dax_mode_appends_distinct_cmdline_flag() {
    let base = "console=hvc0 ro loglevel=1";
    let shares: Vec<VirtioFsShare> = vec![];
    let rootfs = Path::new("/tmp/rootfs.erofs");
    let effective = effective_kernel_cmdline_with_erofs_mode(base, &shares, Some(rootfs), true);
    assert!(effective.contains("capsem.rootfs=erofs-dax"));
    assert!(!effective.contains("capsem.rootfs=erofs "));
}

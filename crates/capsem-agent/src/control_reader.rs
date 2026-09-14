//! Host control frames and their per-connection guest owners.
use crate::control_writer::{CtrlSender, PendingResponses};
use crate::snapshot::{freeze_system_filesystem, thaw_system_filesystem, SYSTEM_FS_MOUNT};
use crate::{
    delete_nofollow, port_bridge, read_nofollow, recv_host_msg, run_exec, set_system_clock, set_winsize, shutdown,
    write_nofollow, GUEST_WORKSPACE_ROOT,
};
use capsem_proto::{validate_file_path_safe, GuestToHost, HostToGuest, MAX_FRAME_SIZE, SHUTDOWN_GRACE_SECS};
use nix::{libc, unistd::Pid};
use std::{os::unix::io::RawFd, thread};

#[allow(clippy::too_many_arguments)]
pub(crate) fn control_loop(
    control_fd: RawFd,
    master_fd: RawFd,
    child_pid: Pid,
    boot_env: &[(String, String)],
    ctrl_tx: CtrlSender,
    exec_inflight: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
    exec_done: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, i32>>>,
    pending_responses: PendingResponses,
) {
    let mut publications: Option<port_bridge::Bridge> = None;
    loop {
        match recv_host_msg(control_fd) {
            Ok(HostToGuest::ConnectPort { flow, port }) => {
                let result = (|| {
                    if publications.is_none() {
                        publications = Some(port_bridge::Bridge::new(ctrl_tx.clone())?);
                    }
                    publications.as_mut().unwrap().connect(flow, port)
                })();
                if let Err(error) = result {
                    tracing::debug!(connection_id = flow.id, generation = flow.generation, %error, "guest publication refused");
                }
            }
            Ok(HostToGuest::AbortPorts { flows }) => {
                if let Some(bridge) = publications.as_mut() {
                    if let Err(error) = bridge.abort(&flows) {
                        tracing::warn!(%error, "invalid guest publication abort");
                    }
                }
            }
            Ok(HostToGuest::AckReply { id }) => {
                // Host received the corresponding ackable response;
                // drop it from the replay buffer so the next rekey
                // does not re-send it. No-op if already removed (e.g.
                // duplicate AckReply from a replayed response that
                // actually did land twice).
                pending_responses.lock().unwrap().remove(&id);
            }
            Ok(HostToGuest::PortCloseAck { flow }) => ctrl_tx.network.acknowledge(flow),
            Ok(HostToGuest::Resize { cols, rows }) => {
                eprintln!("[capsem-agent] resize: {cols}x{rows}");
                set_winsize(master_fd, cols, rows);
                // Send SIGWINCH to the foreground process group.
                unsafe {
                    let mut pgrp: libc::pid_t = 0;
                    if libc::ioctl(master_fd, libc::TIOCGPGRP, &mut pgrp) == 0 && pgrp > 0 {
                        libc::kill(-pgrp, libc::SIGWINCH);
                    }
                }
            }
            Ok(HostToGuest::Ping { epoch_secs }) => {
                if epoch_secs > 0 {
                    set_system_clock(epoch_secs);
                }
                if ctrl_tx.send(GuestToHost::Pong).is_err() {
                    eprintln!("[capsem-agent] control write channel closed");
                    break;
                }
            }
            Ok(HostToGuest::Shutdown) => {
                eprintln!("[capsem-agent] received Shutdown from host");
                // Flag first: the bridge must not end the connection when
                // the shell exits, or the report below is lost with it.
                ctrl_tx.shutdown.request();
                drop(publications.take());
                let end = shutdown::end_terminal_shell(child_pid, std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS));
                eprintln!("[capsem-agent] shutdown: {end}");
                // Tell the host it can stop the VM now rather than at the
                // end of its own timer, and wait for the writer to confirm
                // the bytes went out. A miss means the host is on its timer.
                if ctrl_tx.send(GuestToHost::ShutdownComplete).is_err()
                    || !ctrl_tx.shutdown.wait_reported(shutdown::REPORT_WRITE_WAIT)
                {
                    eprintln!("[capsem-agent] shutdown report not delivered; host will time out");
                }
                break;
            }
            Ok(HostToGuest::Exec { id, command }) => {
                // Ack immediately on receipt -- before any processing or
                // dedup -- so the host bridge can clear this id from its
                // pending-ack map and stop re-replaying it on rekey.
                // The host writes Exec into the pending map *before*
                // sending; if our Ack is itself lost, the bridge
                // re-sends Exec on the next conn, we re-ack: idempotent.
                if ctrl_tx.send(GuestToHost::Ack { id }).is_err() {
                    break;
                }
                // Three states for `id`:
                //   - Done (in exec_done):    cached exit_code; replay
                //                             ExecDone so the host's
                //                             j_rx resolves even when
                //                             the original was lost on
                //                             return.
                //   - In-flight (in_inflight): original is still
                //                              running; ignore the
                //                              retry, the original
                //                              will send ExecDone.
                //   - Fresh:                   record as inflight, run.
                let cached = {
                    let done = exec_done.lock().unwrap();
                    done.get(&id).copied()
                };
                if let Some(exit_code) = cached {
                    eprintln!(
                        "[capsem-agent] exec[{id}] duplicate (already done, exit={exit_code}); replaying ExecDone"
                    );
                    if ctrl_tx.send(GuestToHost::ExecDone { id, exit_code }).is_err() {
                        break;
                    }
                    continue;
                }
                let inserted = exec_inflight.lock().unwrap().insert(id);
                if !inserted {
                    eprintln!("[capsem-agent] exec[{id}] duplicate (still inflight); ignoring");
                    continue;
                }
                eprintln!("[capsem-agent] exec[{id}]: {command}");
                let boot_env = boot_env.to_vec();
                let tx = ctrl_tx.clone();
                let inflight = std::sync::Arc::clone(&exec_inflight);
                let done = std::sync::Arc::clone(&exec_done);
                thread::spawn(move || {
                    let outcome = run_exec(&tx, id, &command, &boot_env);
                    // Record completion *before* dropping inflight so a
                    // host replay that arrives between the two
                    // unlocks cannot miss both maps. Only cache real
                    // child exits -- transport failures (vsock connect
                    // exhausted retries) are transient; caching 126
                    // would poison every subsequent replay
                    // even after the transport recovered (Bug C).
                    if outcome.should_cache() {
                        let mut d = done.lock().unwrap();
                        if d.len() >= 4096 {
                            d.clear();
                        }
                        d.insert(id, outcome.exit_code());
                    }
                    inflight.lock().unwrap().remove(&id);
                });
            }
            Ok(HostToGuest::FileWrite { id, path, data, mode }) => {
                // Ack on receipt so the host bridge clears the
                // pending-ack entry. No dedup: write_nofollow over the
                // same path with the same bytes is idempotent, and
                // re-acking lets the host recover from a lost FileOpDone.
                if ctrl_tx.send(GuestToHost::Ack { id }).is_err() {
                    break;
                }
                eprintln!("[capsem-agent] FileWrite {path} ({} bytes)", data.len());
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error {
                        id,
                        message: format!("FileWrite rejected: {e}"),
                    }
                } else if let Err(e) = write_nofollow(&path, &data, mode) {
                    GuestToHost::Error {
                        id,
                        message: format!("failed to write {path}: {e}"),
                    }
                } else {
                    GuestToHost::FileOpDone { id }
                };
                if ctrl_tx.send(msg).is_err() {
                    break;
                }
            }
            Ok(HostToGuest::FileRead { id, path }) => {
                // Ack on receipt so the host bridge clears the
                // pending-ack entry. No dedup: re-reading is idempotent
                // and lets the host recover when FileContent was lost on
                // the return path.
                if ctrl_tx.send(GuestToHost::Ack { id }).is_err() {
                    break;
                }
                eprintln!("[capsem-agent] FileRead {path}");
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error {
                        id,
                        message: format!("FileRead rejected: {e}"),
                    }
                } else {
                    match read_nofollow(&path, MAX_FRAME_SIZE as usize) {
                        Ok(data) => {
                            let reply = GuestToHost::FileContent {
                                id,
                                path: path.clone(),
                                data,
                            };
                            if capsem_proto::guest_msg_fits_frame(&reply) {
                                reply
                            } else {
                                GuestToHost::Error {
                                    id,
                                    message: format!(
                                        "failed to read {path}: file too large for one control frame ({MAX_FRAME_SIZE} bytes)"
                                    ),
                                }
                            }
                        }
                        Err(e) => GuestToHost::Error {
                            id,
                            message: format!("failed to read {path}: {e}"),
                        },
                    }
                };
                if ctrl_tx.send(msg).is_err() {
                    break;
                }
            }
            Ok(HostToGuest::FileDelete { id, path }) => {
                // Ack on receipt so the host bridge clears the
                // pending-ack entry. No dedup: a second delete of an
                // already-removed file returns ENOENT, which we coerce
                // to FileOpDone so a retried delete (whose original
                // FileOpDone was lost on return) ends up as a success
                // rather than an Error.
                if ctrl_tx.send(GuestToHost::Ack { id }).is_err() {
                    break;
                }
                eprintln!("[capsem-agent] FileDelete {path}");
                let ws = std::path::Path::new(GUEST_WORKSPACE_ROOT);
                let msg = if let Err(e) = validate_file_path_safe(&path, ws) {
                    GuestToHost::Error {
                        id,
                        message: format!("FileDelete rejected: {e}"),
                    }
                } else {
                    match delete_nofollow(&path) {
                        Ok(()) => GuestToHost::FileOpDone { id },
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => GuestToHost::FileOpDone { id },
                        Err(e) => GuestToHost::Error {
                            id,
                            message: format!("failed to delete {path}: {e}"),
                        },
                    }
                };
                if ctrl_tx.send(msg).is_err() {
                    break;
                }
            }
            Ok(HostToGuest::PrepareSnapshot) => {
                drop(publications.take());
                // Flush guest dirty pages out to the system-overlay
                // virtio-blk device (/dev/vdb) before host save_state /
                // host-side APFS clonefile runs. sync() drains the page
                // cache; BLKFLSBUF + fsync flush the block device's own
                // buffers. Finally freeze the ext4 upper so no new write can
                // race between this acknowledgement and the host pausing all
                // vCPUs. The host then captures a coherent file.
                //
                // /mnt/shared/system/rootfs.img is no longer the guest's
                // data path -- the guest only writes through /dev/vdb
                // -- so no FUSE_FSYNC over VirtioFS is needed here.
                eprintln!("[capsem-agent] PrepareSnapshot: syncing and flushing /dev/vdb");
                unsafe {
                    libc::sync();
                }

                let fd = unsafe { libc::open(c"/dev/vdb".as_ptr(), libc::O_RDWR) };
                if fd >= 0 {
                    const BLKFLSBUF: i32 = 0x1261;
                    unsafe {
                        if libc::ioctl(fd, BLKFLSBUF.try_into().unwrap()) != 0 {
                            eprintln!(
                                "[capsem-agent] ioctl(BLKFLSBUF) failed: {}",
                                std::io::Error::last_os_error()
                            );
                        }
                        if libc::fsync(fd) != 0 {
                            eprintln!("[capsem-agent] fsync failed: {}", std::io::Error::last_os_error());
                        }
                        libc::close(fd);
                    }
                } else {
                    eprintln!(
                        "[capsem-agent] failed to open /dev/vdb for flush: {}",
                        std::io::Error::last_os_error()
                    );
                }

                if let Err(e) = freeze_system_filesystem() {
                    eprintln!("[capsem-agent] PrepareSnapshot: failed to freeze system filesystem: {e}");
                    // Do not acknowledge an unsafe snapshot. The host timeout
                    // path sends Unfreeze and fails the suspend closed.
                    continue;
                }

                if ctrl_tx.send(GuestToHost::SnapshotReady).is_err() {
                    let _ = thaw_system_filesystem();
                    break;
                }
            }
            Ok(HostToGuest::Unfreeze) => {
                eprintln!("[capsem-agent] Unfreeze: thawing {SYSTEM_FS_MOUNT}");
                if let Err(e) = thaw_system_filesystem() {
                    eprintln!("[capsem-agent] failed to thaw system filesystem: {e}");
                }
            }
            Ok(msg) => {
                eprintln!("[capsem-agent] unhandled control message: {msg:?}");
            }
            Err(e) => {
                eprintln!("[capsem-agent] control channel error: {e}");
                break;
            }
        }
    }
}

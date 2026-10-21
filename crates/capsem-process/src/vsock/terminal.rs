//! The session terminal's transport. Byte interpretation belongs to core.
use super::*;

pub(super) fn spawn(
    terminal_output: Arc<capsem_core::terminal::SessionTerminal>,
    mut terminal_rekey_rx: mpsc::Receiver<Arc<VsockConnection>>,
    mut term_in_rx: mpsc::Receiver<Vec<u8>>,
    pty_log: Option<Arc<capsem_core::pty_log::PtyLog>>,
    serial_log: capsem_foundation::telemetry::SharedLogWriter,
) {
    // 1. Stable Terminal Bridge (Read + Write)
    let term_out = terminal_output;
    let pty_log_out = pty_log;
    let (serial_log_tx, serial_log_handle) = spawn_serial_log_writer(move || Ok(serial_log));

    tokio::spawn(async move {
        let mut current_conn = terminal_rekey_rx.recv().await;
        loop {
            let conn = match current_conn.take() {
                Some(c) => c,
                None => match terminal_rekey_rx.recv().await {
                    Some(c) => c,
                    None => break,
                },
            };

            info!("terminal bridge: active");
            let Some(mut reader) = clone_fd(&conn, "duplicate-terminal-vsock-reader") else {
                continue;
            };
            let Some(mut writer) = clone_fd(&conn, "duplicate-terminal-vsock-writer") else {
                continue;
            };
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(128);

            // Blocking read thread for this specific FD
            let read_handle = std::thread::spawn(move || {
                let mut buf = vec![0u8; 65536];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            });

            let mut coalesce = capsem_core::vm::vsock::CoalesceBuffer::new();
            loop {
                tokio::select! {
                    // Incoming from Guest
                    res = rx.recv() => {
                        match res {
                            Some(data) => {
                                coalesce.push(&data);
                                let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(coalesce.window_ms());
                                while !coalesce.is_full() {
                                    match tokio::time::timeout_at(deadline, rx.recv()).await {
                                        Ok(Some(d)) => { coalesce.push(&d); }
                                        _ => break,
                                    }
                                }
                                let batch = coalesce.flush_to(<[u8]>::to_vec);
                                let replies = term_out.publish(&batch);
                                if !replies.is_empty() && writer.write_all(&replies).is_err() { break; }
                                if let Some(ref pl) = pty_log_out { pl.record_output(&batch); }
                                let _ = serial_log_tx.send(batch).await;

                            }
                            None => break, // FD closed
                        }
                    }
                    // Outgoing to Guest (Terminal Input)
                    Some(data) = term_in_rx.recv() => {
                        if writer.write_all(&data).is_err() { break; }
                        let _ = writer.flush();
                    }
                    // Reconnection arriving
                    Some(new_conn) = terminal_rekey_rx.recv() => {
                        current_conn = Some(new_conn);
                        break;
                    }
                }
            }
            // The reader thread is parked in read() on a dup of this socket.
            // Closing our fd does not wake it and a half-open post-restore
            // socket never EOFs, so joining here parked a runtime worker for
            // good and the new connection was never activated. Shut the
            // socket down so the read returns, then join off the runtime.
            if let Err(error) = conn.shutdown_both() {
                if error.kind() != std::io::ErrorKind::NotConnected {
                    warn!(
                        operation = "shutdown-terminal-vsock",
                        errno = error.raw_os_error(),
                        error = %error,
                        "failed to wake terminal reader"
                    );
                }
            }
            let _ = tokio::task::spawn_blocking(move || read_handle.join()).await;
        }
        drop(serial_log_tx);
        let _ = tokio::task::spawn_blocking(move || serial_log_handle.join()).await;
    });
}

//! Bounded guest boot handshakes and reconnect classification.
use super::*;

/// Run the boot handshake on an already-accepted control fd.
///
/// Must be invoked from `spawn_blocking`: all I/O here is synchronous on
/// a `std::fs::File` wrapper over the vsock fd, and doing it inline on a
/// tokio worker starves the runtime under multi-VM boot contention.
///
/// `.context()` (not `map_err(anyhow!)`) is used throughout so the
/// underlying `std::io::Error` stays in the error source chain, which
/// `is_retryable_handshake_error` downcasts to decide whether to retry.
pub(super) fn perform_handshake(
    fd: &mut std::fs::File,
    is_restore: bool,
    env: &[(String, String)],
    conf: Option<capsem_core::net::policy_config::GuestConfig>,
) -> Result<()> {
    read_control_msg(fd).context("initial Ready read failed")?;
    if is_restore {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let traceparent = capsem_foundation::telemetry::current_parent_traceparent().to_string();
        write_control_msg(
            fd,
            &HostToGuest::BootConfig {
                epoch_secs: epoch,
                traceparent,
            },
        )
        .context("restore BootConfig write failed")?;
        // Re-inject timezone in case host TZ changed since suspend. These
        // writes are best-effort: failing to reset the guest clock is not
        // itself a handshake failure.
        if let Ok(link) = std::fs::read_link("/etc/localtime") {
            if let Some(s) = link.to_str() {
                if let Some(idx) = s.find("/zoneinfo/") {
                    let tz = &s[idx + "/zoneinfo/".len()..];
                    let _ = write_control_msg(
                        fd,
                        &HostToGuest::SetEnv {
                            key: "TZ".into(),
                            value: tz.to_string(),
                        },
                    );
                    if let Ok(tz_data) = std::fs::read("/etc/localtime") {
                        let _ = write_control_msg(
                            fd,
                            &HostToGuest::FileWrite {
                                id: 0,
                                path: "/etc/localtime".into(),
                                data: tz_data,
                                mode: 0o644,
                            },
                        );
                    }
                }
            }
        }
        write_control_msg(fd, &HostToGuest::BootConfigDone).context("restore BootConfigDone write failed")?;
    } else {
        capsem_core::send_boot_config(fd, env, conf).context("send_boot_config failed")?;
    }
    read_control_msg(fd).context("BootReady read failed")?;
    Ok(())
}

/// Collect a terminal+control pair from the vsock accept stream.
///
/// Auxiliary connections (MITM proxy, audit, DNS) that race ahead
/// of the pair are parked in `deferred_conns` so the caller can hand
/// them to the long-running dispatcher once the handshake succeeds.
pub(super) async fn collect_terminal_control_pair(
    vsock_rx: &mut mpsc::UnboundedReceiver<VsockConnection>,
    deferred_conns: &mut Vec<VsockConnection>,
) -> Result<(VsockConnection, VsockConnection)> {
    let mut terminal = None;
    let mut control = None;
    while terminal.is_none() || control.is_none() {
        let Some(conn) = vsock_rx.recv().await else {
            anyhow::bail!("vsock channel closed before terminal/control pair arrived");
        };
        match conn.port {
            proto::VSOCK_PORT_TERMINAL => terminal = Some(conn),
            proto::VSOCK_PORT_CONTROL => control = Some(conn),
            proto::VSOCK_PORT_SNI_PROXY | proto::VSOCK_PORT_AUDIT | proto::VSOCK_PORT_DNS_PROXY => {
                deferred_conns.push(conn);
            }
            _ => {}
        }
    }
    Ok((terminal.unwrap(), control.unwrap()))
}

/// Classify a handshake error as retryable.
///
/// All cover the same observed pattern: Apple VZ tears the post-restoreState
/// vsock conn down between the guest sending one frame and the next, leaving
/// the host with a dead fd. The kind we get depends on which side closes
/// first and how:
///   - `BrokenPipe` / `ConnectionReset` -- guest's end shut down hard.
///   - `UnexpectedEof` -- guest closed cleanly mid-frame; we get EOF on
///     `read_exact`. Empirically this is the dominant kind under heavy
///     suspend/resume churn (see commit history of this file).
///
/// Retrying drops the dead pair and waits for the guest's reconnect loop to
/// open a fresh terminal+control pair, then re-runs the handshake. Capped
/// at `HANDSHAKE_RETRY_MAX` so a genuinely broken guest fails fast.
pub(super) fn is_retryable_handshake_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            matches!(
                io.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::UnexpectedEof
            )
        })
    })
}

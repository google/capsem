//! Project-wide macros. Currently just `try_send!` for IPC/vsock channels.

/// Send on any `Result`-returning channel API and `tracing::warn!` (instead of
/// silently dropping) if the send fails. Replaces every `let _ = X.send(...)`
/// or `let _ = X.send(...).await` on IPC/vsock channels in the project.
///
/// The `target` is hard-coded to `"ipc"` so a single
/// `RUST_LOG=ipc=warn` flips on "show me every dropped message" without
/// chatter from other subsystems.
///
/// Works for `tokio::sync::mpsc::Sender::send(...).await`,
/// `tokio::sync::mpsc::UnboundedSender::send(...)`,
/// `tokio::sync::broadcast::Sender::send(...)`,
/// `tokio::sync::oneshot::Sender::send(...)`,
/// `std::sync::mpsc::Sender::send(...)`, and any other `Result`-returning
/// send-shaped expression. The macro itself does not add `.await` -- pass
/// the fully-formed expression and the macro just inspects its `Result`.
///
/// Usage:
/// ```
/// let (tx, rx) = std::sync::mpsc::channel();
/// capsem_core::try_send!("state_change", tx.send(7));
/// assert_eq!(rx.recv().unwrap(), 7);
/// ```
///
/// Cleanup paths where a closed receiver is the documented design (e.g. a
/// `oneshot::Sender::send` whose `Receiver` was cancelled) keep the bare
/// `let _ = X.send(...)` and add a trailing comment `// channel-closed-ok: <reason>`
/// so the audit grep can exclude them.
#[macro_export]
macro_rules! try_send {
    ($channel:expr, $send_expr:expr) => {{
        if let ::std::result::Result::Err(__e) = $send_expr {
            ::tracing::warn!(
                target: "ipc",
                channel = $channel,
                error = ?__e,
                "dropped message: send failed (channel closed or full)"
            );
        }
    }};
}

#[cfg(test)]
mod tests;

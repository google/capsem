//! Shell-exit cleanup helpers.
//!
//! Extracted so the contract can be unit-tested without standing up a real
//! VM or IPC channel. See `tests.rs` for the invariants this module is
//! pinning -- in short, "what `capsem shell` writes to the user's terminal
//! after the loop exits, and what it does NOT".

use tokio::io::AsyncWriteExt;

/// Bytes we write to stdout right before letting the `RawModeGuard` in
/// `run_shell` restore termios.
///
/// - `\x1b[0m` -- SGR reset (clear bold/colors/inverse). Without this a
///   guest that ended mid-color paints the parent shell prompt the wrong color.
/// - `\x1b[?25h` -- show cursor. Guests sometimes hide it (e.g. fullscreen
///   TUIs) and crash before showing it again.
/// - `\r\n` -- explicit CRLF so the next prompt starts at column 0
///   even if the guest left the cursor mid-line.
///
/// Deliberately does NOT include alt-screen toggles or screen clears --
/// those would erase the user's scrollback. See `tests.rs` for the guard
/// rails that keep accidental additions out.
pub const TERMINAL_RESET_SEQUENCE: &[u8] = b"\x1b[0m\x1b[?25h\r\n";

/// Write the reset sequence to the user's stdout (only when on a tty;
/// on a pipe or file, escape codes would just clutter the output).
///
/// Best-effort: errors are swallowed because by the time we hit this path
/// we're already exiting and there is nothing useful to do with a failure.
pub async fn reset_user_terminal(is_tty: bool) {
    if !is_tty {
        return;
    }
    let mut stdout = tokio::io::stdout();
    let _ = stdout.write_all(TERMINAL_RESET_SEQUENCE).await;
    let _ = stdout.flush().await;
}

/// Re-export of the canonical detector in `capsem_proto`. Kept under the
/// `shell_exit` namespace because that's the consumer the tests cover and
/// the documentation comments are co-located.
pub use capsem_proto::looks_like_ipc_frame as looks_like_msgpack_ipc_frame;

#[cfg(test)]
mod tests;

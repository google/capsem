use axum::serve::{ListenerExt, TapIo};
use tokio::net::{TcpListener, TcpStream};

fn disable_nagle(stream: &mut TcpStream) {
    if let Err(error) = stream.set_nodelay(true) {
        tracing::warn!(%error, "failed to enable TCP_NODELAY on gateway connection");
    }
}

/// Disable Nagle buffering on every accepted local gateway connection.
///
/// Gateway responses are small control-plane messages. Under sustained
/// keep-alive traffic, coalescing their header and body writes can interact
/// with delayed acknowledgements and introduce repeatable ~40 ms stalls.
/// Configure the accepted stream at the listener boundary so HTTP, WebSocket,
/// and future routes all inherit the same low-latency transport contract.
pub(crate) fn low_latency(listener: TcpListener) -> TapIo<TcpListener, fn(&mut TcpStream)> {
    listener.tap_io(disable_nagle)
}

#[cfg(test)]
#[path = "listener/tests.rs"]
mod tests;

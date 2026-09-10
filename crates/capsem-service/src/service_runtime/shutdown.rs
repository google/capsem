//! Drain service replies before terminating the gateway that forwards them.

use std::{future::Future, io, time::Duration};

use axum::Router;
use tokio::{net::UnixListener, process::Child, sync::oneshot};

const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn serve(
    listener: UnixListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    drain(listener, app, shutdown, DRAIN_TIMEOUT).await
}

async fn drain(
    listener: UnixListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
    timeout: Duration,
) -> io::Result<()> {
    let (started, stopped) = oneshot::channel();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown.await;
        let _ = started.send(());
    });
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => return result,
        _ = stopped => {}
    }
    match tokio::time::timeout(timeout, server).await {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!("service response drain timed out");
            Ok(())
        }
    }
}

pub(super) async fn stop_companions(children: Vec<Child>) {
    stop_companions_with_timeout(children, DRAIN_TIMEOUT).await;
}

async fn stop_companions_with_timeout(children: Vec<Child>, timeout: Duration) {
    // The gateway's SIGTERM handler stops accepting and drains forwarded
    // replies. Keep its parent alive until it exits: parent-watch otherwise
    // terminates it before that final TCP response reaches the SDK.
    for mut child in children {
        if let Some(pid) = child.id() {
            super::process_control::send_or_log(
                pid,
                super::process_control::Signal::Terminate,
                "drain-service-companion",
            );
        }
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(_)) => {}
            result => {
                tracing::warn!(?result, pid = child.id(), "forcing companion shutdown");
                let _ = child.kill().await;
            }
        }
    }
}

#[cfg(test)]
mod tests;

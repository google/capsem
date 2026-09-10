//! Per-control-connection owner for guest publication setup and streams.
use crate::vsock_io::AsyncVsock;
use std::io;
use std::net::TcpStream;
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;

mod setup;

pub struct Bridge {
    runtime: Runtime,
    tasks: JoinSet<()>,
    connections: Arc<Semaphore>,
    setups: Arc<Semaphore>,
    stop: watch::Sender<bool>,
}

impl Bridge {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
            tasks: JoinSet::new(),
            connections: Arc::new(Semaphore::new(64)),
            setups: Arc::new(Semaphore::new(8)),
            stop: watch::channel(false).0,
        })
    }

    pub fn connect(&mut self, id: u64, port: u16) -> io::Result<()> {
        if id == 0 || port == 0 {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        self.connect_with(id, move || setup::connect(id, port))
    }

    fn connect_with(
        &mut self,
        id: u64,
        setup: impl FnOnce() -> io::Result<(TcpStream, UnixStream)> + Send + 'static,
    ) -> io::Result<()> {
        if *self.stop.borrow() {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let permit = self
            .connections
            .clone()
            .try_acquire_owned()
            .map_err(|_| io::Error::other("publication connection limit reached"))?;
        while let Some(result) = self.tasks.try_join_next() {
            if let Err(error) = result {
                tracing::error!(%error, "guest publication task failed");
            }
        }
        let setups = self.setups.clone();
        let mut stop = self.stop.subscribe();
        self.tasks.spawn_on(
            async move {
                let result = async {
                    let setup_permit = tokio::select! {
                        biased;
                        _ = stop.changed() => return Ok(()),
                        permit = setups.acquire_owned() => permit.map_err(io::Error::other)?,
                    };
                    // The blocking pool only joins. Namespace changes happen on a
                    // disposable thread, never on a reusable Tokio worker. Once
                    // started, always await its finite setup before cancellation.
                    let endpoints = tokio::task::spawn_blocking(move || {
                        std::thread::Builder::new()
                            .name("capsem-port-connect".into())
                            .spawn(setup)?
                            .join()
                            .map_err(|_| io::Error::other("publication setup panicked"))?
                    })
                    .await
                    .map_err(io::Error::other)?;
                    drop(setup_permit);
                    let (tcp, vsock) = endpoints?;
                    if *stop.borrow() {
                        return Ok(());
                    }
                    tcp.set_nonblocking(true)?;
                    tcp.set_nodelay(true)?;
                    let mut tcp = tokio::net::TcpStream::from_std(tcp)?;
                    let mut vsock = AsyncVsock::new(vsock.into_raw_fd())?;
                    tokio::select! {
                        biased;
                        _ = stop.changed() => {},
                        outcome = capsem_foundation::unix::router_stream::copy(&mut tcp, &mut vsock,
                            capsem_foundation::unix::router_stream::Limits::default()) => {
                            tracing::debug!(connection_id = id, reason = ?outcome.reason,
                                from_source = outcome.from_source, to_source = outcome.to_source,
                                error = ?outcome.error, "guest router stream ended");
                        }
                    }
                    Ok::<_, io::Error>(())
                }
                .await;
                if let Err(error) = result {
                    tracing::debug!(connection_id = id, %error, "guest publication refused");
                }
                drop(permit);
            },
            self.runtime.handle(),
        );
        Ok(())
    }

    fn shutdown(&mut self) {
        self.stop.send_replace(true);
        self.runtime.block_on(async {
            while let Some(result) = self.tasks.join_next().await {
                if let Err(error) = result {
                    tracing::error!(%error, "guest publication task failed during drain");
                }
            }
        });
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;

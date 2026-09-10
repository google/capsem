//! Bounded container TCP setup; namespace changes stay on disposable threads.
use crate::vsock_io::{self, AsyncVsock, VSOCK_HOST_CID};
use std::io::{self, Write};
use std::net::TcpStream;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::sync::{Arc, OnceLock};
use tokio::runtime::Runtime;
use tokio::sync::Semaphore;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static CONNECTIONS: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub fn connect(id: u64, port: u16) -> io::Result<()> {
    if id == 0 || port == 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    if RUNTIME.get().is_none() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let _ = RUNTIME.set(runtime);
    }
    let runtime = RUNTIME.get().unwrap().handle().clone();
    let permit = CONNECTIONS
        .get_or_init(|| Arc::new(Semaphore::new(128)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| io::Error::other("publication connection limit reached"))?;
    std::thread::Builder::new()
        .name("capsem-port-connect".into())
        .spawn(move || {
            let result = (|| {
                let fd = vsock_io::vsock_connect(VSOCK_HOST_CID, capsem_proto::VSOCK_PORT_PUBLICATION)?;
                // SAFETY: vsock_connect returns a newly owned fd or an error.
                let mut vsock = std::os::unix::net::UnixStream::from(unsafe { OwnedFd::from_raw_fd(fd) });
                let tcp = container_tcp(port);
                let mut header = [0; 9];
                header[..8].copy_from_slice(&id.to_be_bytes());
                header[8] = u8::from(tcp.is_ok());
                vsock.write_all(&header)?;
                let tcp = tcp?;
                tcp.set_nonblocking(true)?;
                tcp.set_nodelay(true)?;
                runtime.spawn(async move {
                    let _permit = permit;
                    let result = async {
                        let mut vsock = AsyncVsock::new(vsock.into_raw_fd())?;
                        let mut tcp = tokio::net::TcpStream::from_std(tcp)?;
                        tokio::io::copy_bidirectional(&mut tcp, &mut vsock).await?;
                        Ok::<_, io::Error>(())
                    }
                    .await;
                    if let Err(error) = result {
                        eprintln!("[capsem-agent] published connection {id}: {error}");
                    }
                });
                Ok::<_, io::Error>(())
            })();
            if let Err(error) = result {
                eprintln!("[capsem-agent] publication setup {id}: {error}");
            }
        })?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn container_tcp(port: u16) -> io::Result<TcpStream> {
    use std::net::{Ipv4Addr, SocketAddr};
    use std::os::fd::AsRawFd;
    use std::time::Duration;
    let pid: u32 = std::fs::read_to_string("/var/tmp/capsem-container/workload.pid")?
        .trim()
        .parse()
        .map_err(|_| io::Error::other("invalid container pid"))?;
    if pid <= 1 {
        return Err(io::Error::other("invalid container pid"));
    }
    let namespace = std::fs::File::open(format!("/proc/{pid}/ns/net"))?;
    // SAFETY: only this short-lived setup thread changes namespace. It creates
    // one TCP socket then exits; reusable runtime workers never call setns.
    if unsafe { nix::libc::setns(namespace.as_raw_fd(), nix::libc::CLONE_NEWNET) } != 0 {
        return Err(io::Error::last_os_error());
    }
    TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_secs(3))
}

#[cfg(not(target_os = "linux"))]
fn container_tcp(_: u16) -> io::Result<TcpStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "container namespace requires Linux",
    ))
}

//! Disposable namespace worker; all socket operations share one setup deadline.
use crate::vsock_io::{self, VSOCK_HOST_CID};
use std::io::{self, Write};
use std::net::TcpStream;
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

pub(super) fn connect(id: u64, port: u16) -> io::Result<(TcpStream, UnixStream)> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let fd = vsock_io::vsock_connect_with_timeout(
        VSOCK_HOST_CID,
        capsem_proto::VSOCK_PORT_PUBLICATION,
        remaining(deadline)?,
    )?;
    // SAFETY: vsock_connect returns a new owned descriptor.
    let mut vsock = unsafe { UnixStream::from_raw_fd(fd) };
    capsem_foundation::unix::fd::set_stream_buffers(
        vsock.as_fd(),
        capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE,
    )?;
    let tcp = container_tcp(port, remaining(deadline)?).and_then(|tcp| {
        capsem_foundation::unix::fd::set_stream_buffers(
            tcp.as_fd(),
            capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE,
        )?;
        Ok(tcp)
    });
    let mut header = [0; 9];
    header[..8].copy_from_slice(&id.to_be_bytes());
    header[8] = u8::from(tcp.is_ok());
    vsock.set_write_timeout(Some(remaining(deadline)?))?;
    vsock.write_all(&header)?;
    Ok((tcp?, vsock))
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
}

#[cfg(target_os = "linux")]
fn container_tcp(port: u16, timeout: Duration) -> io::Result<TcpStream> {
    use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
    use nix::libc;
    use std::io::Read;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::os::fd::AsRawFd;
    let directory = ContainedDir::open_root(std::path::Path::new("/var/tmp/capsem-container"))?;
    let file = directory.open_file(std::ffi::OsStr::new("workload.pid"), ContainedOpenOptions::read_only())?;
    drop(directory);
    let mut contents = String::new();
    file.take(12).read_to_string(&mut contents)?;
    if contents.len() >= 12 {
        return Err(io::Error::other("container pid exceeds size limit"));
    }
    let pid: u32 = contents
        .trim()
        .parse()
        .map_err(|_| io::Error::other("invalid container pid"))?;
    if pid <= 1 {
        return Err(io::Error::other("invalid container pid"));
    }
    let namespace = std::fs::File::open(format!("/proc/{pid}/ns/net"))?;
    // SAFETY: caller owns a disposable setup thread; reusable workers never setns.
    if unsafe { libc::setns(namespace.as_raw_fd(), libc::CLONE_NEWNET) } != 0 {
        return Err(io::Error::last_os_error());
    }
    TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), timeout)
}

#[cfg(not(target_os = "linux"))]
fn container_tcp(_: u16, _: Duration) -> io::Result<TcpStream> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "container namespace requires Linux",
    ))
}

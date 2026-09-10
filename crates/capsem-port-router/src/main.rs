use capsem_foundation::unix::{fd, router_sandbox};
use capsem_port_router::Grant;
use clap::Parser;
use std::io;
use std::os::fd::AsFd;

#[derive(Parser)]
#[command(version, about = "Confined TCP publication companion")]
struct Args {
    #[arg(long)]
    parent_pid: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: process entry, before constructing any descriptor owner or thread.
    unsafe { router_sandbox::close_inherited_descriptors()? };
    let args = Args::parse();
    capsem_guard::watch_parent_or_exit(Some(args.parent_pid))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let stdin = std::io::stdin();
        let socket = std::os::unix::net::UnixStream::from(fd::duplicate(stdin.as_fd())?);
        socket.set_nonblocking(true)?;
        let grants = capsem_foundation::unix::router_channel::Receiver::<Grant>::new(socket.try_clone()?)?;
        let Grant::Listen { socket: listener } = grants.recv().await? else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "router requires listener grant",
            ));
        };
        let listener = std::net::TcpListener::from(listener.into_inner());
        let address = listener.local_addr()?;
        if !address.ip().is_loopback() {
            return Err(io::Error::other("router listener must be loopback"));
        }
        listener.set_nonblocking(true)?;
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let events = tokio::net::UnixStream::from_std(socket)?;
        router_sandbox::confine(address.port())?;
        capsem_port_router::relay(listener, grants, events).await
    })?;
    Ok(())
}

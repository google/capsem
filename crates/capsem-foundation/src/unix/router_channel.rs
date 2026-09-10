//! Descriptor channels with deregistration-before-close ownership.
//!
//! tokio-unix-ipc 0.4 closes its RawFd before dropping AsyncFd. On Linux,
//! duplicated sockets keep that epoll registration alive; its freed readiness
//! pointer can then hang or panic the runtime. Its IntoRawFd path deregisters
//! first. Never create and immediately drop an unused half of a typed channel.
use serde::{de::DeserializeOwned, Serialize};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

pub struct Sender(Option<tokio_unix_ipc::RawSender>);
pub struct Receiver<T>(Option<tokio_unix_ipc::Receiver<T>>);

fn register<T>(stream: UnixStream, build: impl FnOnce(UnixStream) -> io::Result<T>) -> io::Result<T> {
    stream.set_nonblocking(true)?;
    let raw = stream.as_raw_fd();
    let result = build(stream);
    if result.is_err() {
        // SAFETY: 0.4 transfers the fd before AsyncFd registration; failure
        // leaves it unowned. Only this failure branch closes it.
        drop(unsafe { OwnedFd::from_raw_fd(raw) });
    }
    result
}

impl Sender {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        register(stream, tokio_unix_ipc::RawSender::from_std).map(|raw| Self(Some(raw)))
    }
    pub async fn send(&self, bytes: &[u8], fds: &[i32]) -> io::Result<usize> {
        self.0.as_ref().unwrap().send(bytes, fds).await
    }
}

impl<T: Serialize + DeserializeOwned> Receiver<T> {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        register(stream, tokio_unix_ipc::RawReceiver::from_std).map(|raw| Self(Some(raw.into())))
    }
    pub async fn recv(&self) -> io::Result<T> {
        self.0.as_ref().unwrap().recv().await
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        let raw = self.0.take().unwrap().into_raw_fd();
        // SAFETY: IntoRawFd deregistered and transferred sole ownership.
        drop(unsafe { OwnedFd::from_raw_fd(raw) });
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let raw = self.0.take().unwrap().into_raw_fd();
        // SAFETY: IntoRawFd deregistered and transferred sole ownership.
        drop(unsafe { OwnedFd::from_raw_fd(raw) });
    }
}

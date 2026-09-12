//! The typed IPC transport between the service, the VM owner and the CLI.
//!
//! One module builds every channel so the descriptor rules live once.
//! `tokio_unix_ipc` closes a channel's descriptor in its `Drop` before the
//! `AsyncFd` that holds it drops and deregisters that number from the
//! runtime's kqueue or epoll. Between the two, another task on the same
//! runtime can accept a socket that reuses the number and register it, and
//! the stale deregistration then removes the new socket's interest: its
//! `recv` never wakes, whatever the peer writes. On macOS that lost one
//! private admission in about two thousand, the owner sitting in `recv`
//! until the service gave up eight seconds later (gate 20260912-190634,
//! the redis cell of the private path benchmark), and thirty-two churned
//! connections reproduce it in under fifty rounds.
//!
//! [`Sender`] and [`Receiver`] take the descriptor back from the transport
//! before it can close it, so the registration leaves the runtime while the
//! number is still ours, and the socket closes last.

use std::fmt;
use std::io;
use std::ops::Deref;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// The sending half of a typed channel; derefs to the transport's sender.
pub struct Sender<T>(Option<tokio_unix_ipc::Sender<T>>);

/// The receiving half of a typed channel; derefs to the transport's receiver.
pub struct Receiver<T>(Option<tokio_unix_ipc::Receiver<T>>);

/// Register a connected, non-blocking stream with the current runtime as a
/// typed channel sending `S` and receiving `R`.
pub fn channel_from_std<S, R>(stream: UnixStream) -> io::Result<(Sender<S>, Receiver<R>)>
where
    S: Serialize + DeserializeOwned,
    R: Serialize + DeserializeOwned,
{
    let (sender, receiver) = tokio_unix_ipc::channel_from_std(stream)?;
    Ok((Sender(Some(sender)), Receiver(Some(receiver))))
}

/// Deregister the descriptor from the runtime, then close it.
///
/// `into_raw_fd` marks the transport handle as moved out, so its own `Drop`
/// no longer closes the descriptor; the `AsyncFd` inside deregisters as the
/// handle drops, while the number still names our socket. Closing comes
/// last, through an owner that cannot forget.
fn release(handle: impl IntoRawFd) {
    let raw = handle.into_raw_fd();
    // SAFETY: the transport gave up ownership of `raw` in `into_raw_fd`, it is
    // open, and nothing else closes it.
    drop(unsafe { OwnedFd::from_raw_fd(raw) });
}

macro_rules! half {
    ($ty:ident) => {
        impl<T> Deref for $ty<T> {
            type Target = tokio_unix_ipc::$ty<T>;

            fn deref(&self) -> &Self::Target {
                self.0.as_ref().expect("the transport half lives until drop")
            }
        }

        impl<T> fmt::Debug for $ty<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&**self, f)
            }
        }

        impl<T> Drop for $ty<T> {
            fn drop(&mut self) {
                if let Some(inner) = self.0.take() {
                    release(inner);
                }
            }
        }
    };
}

half!(Sender);
half!(Receiver);

#[cfg(test)]
mod tests;

//! Fixed records and owned SCM_RIGHTS descriptors for the confined router.
//! Cancellation poisons the socket rather than resuming a partial record.
use std::io::{self, ErrorKind};
use std::mem::{size_of, size_of_val, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use tokio::io::unix::AsyncFd;
use tokio::sync::Mutex;

pub const FRAME_SIZE: usize = 10;
pub const MAX_FDS: usize = 2;
pub struct Frame {
    pub bytes: [u8; FRAME_SIZE],
    pub fds: Vec<OwnedFd>,
}

struct Channel {
    io: AsyncFd<UnixStream>,
    operation: Mutex<()>,
}
impl Channel {
    fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            io: AsyncFd::new(stream)?,
            operation: Mutex::new(()),
        })
    }
}
struct Operation<'a> {
    stream: &'a UnixStream,
    complete: bool,
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        if !self.complete {
            if let Err(error) = self.stream.shutdown(std::net::Shutdown::Both) {
                tracing::debug!(%error, "router partial record shutdown");
            }
        }
    }
}
pub struct Sender(Channel);
pub struct Receiver(Channel);
impl Sender {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        Channel::new(stream).map(Self)
    }
    pub async fn send(&self, bytes: &[u8], fds: &[RawFd]) -> io::Result<usize> {
        if bytes.len() != FRAME_SIZE || fds.len() > MAX_FDS {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "invalid router record size or descriptor count",
            ));
        }
        let _lock = self.0.operation.lock().await;
        let mut operation = Operation {
            stream: self.0.io.get_ref(),
            complete: false,
        };
        let mut offset = 0;
        while offset < bytes.len() {
            let mut ready = self.0.io.writable().await?;
            match ready.try_io(|socket| {
                send_record(
                    socket.as_raw_fd(),
                    &bytes[offset..],
                    if offset == 0 { fds } else { &[] },
                )
            }) {
                Ok(Ok(0)) => return Err(ErrorKind::WriteZero.into()),
                Ok(Ok(count)) => offset += count,
                Ok(Err(error)) if error.kind() == ErrorKind::Interrupted => continue,
                Ok(Err(error)) => return Err(error),
                Err(_) => continue,
            }
        }
        operation.complete = true;
        Ok(offset)
    }
}
impl Receiver {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        Channel::new(stream).map(Self)
    }
    pub async fn recv(&self) -> io::Result<Frame> {
        let _lock = self.0.operation.lock().await;
        let mut operation = Operation {
            stream: self.0.io.get_ref(),
            complete: false,
        };
        let mut frame = Frame {
            bytes: [0; FRAME_SIZE],
            fds: Vec::with_capacity(MAX_FDS),
        };
        let mut offset = 0;
        while offset < FRAME_SIZE {
            let mut ready = self.0.io.readable().await?;
            match ready.try_io(|socket| receive_record(socket.as_raw_fd(), &mut frame.bytes[offset..], &mut frame.fds))
            {
                Ok(Ok(0)) => return Err(ErrorKind::UnexpectedEof.into()),
                Ok(Ok(count)) => offset += count,
                Ok(Err(error)) if error.kind() == ErrorKind::Interrupted => continue,
                Ok(Err(error)) => return Err(error),
                Err(_) => continue,
            }
        }
        operation.complete = true;
        Ok(frame)
    }
}
fn send_record(socket: RawFd, bytes: &[u8], fds: &[RawFd]) -> io::Result<usize> {
    // SAFETY: aligned ancillary storage and initialized iovec outlive sendmsg.
    unsafe {
        let mut control = [0usize; 8];
        let mut vector = libc::iovec {
            iov_base: bytes.as_ptr().cast_mut().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = zeroed();
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        if !fds.is_empty() {
            let length = size_of_val(fds) as u32;
            if libc::CMSG_SPACE(length) as usize > size_of_val(&control) {
                return Err(ErrorKind::InvalidInput.into());
            }
            message.msg_control = control.as_mut_ptr().cast();
            message.msg_controllen = libc::CMSG_SPACE(length) as _;
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(length) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(header).cast(), fds.len());
        }
        let count = libc::sendmsg(socket, &message, libc::MSG_NOSIGNAL);
        if count < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(count as usize)
        }
    }
}
fn receive_record(socket: RawFd, bytes: &mut [u8], fds: &mut Vec<OwnedFd>) -> io::Result<usize> {
    // SAFETY: the kernel initializes bounded aligned ancillary storage. Adopt
    // all delivered descriptors before rejecting truncation to avoid leaks.
    unsafe {
        // XNU sockargs caps ancillary storage at MCLBYTES (2048); Linux
        // SCM_MAX_FD is 253. Receive the kernel maximum, then reject >2 FDs.
        // Darwin does not close undisclosed FDs when copyout_control truncates.
        let mut control = [0usize; 256];
        let mut vector = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = zeroed();
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = size_of_val(&control) as _;
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_CMSG_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let flags = 0;
        let count = libc::recvmsg(socket, &mut message, flags);
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut header = libc::CMSG_FIRSTHDR(&message);
        let mut invalid = message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0;
        while !header.is_null() {
            if (*header).cmsg_level != libc::SOL_SOCKET || (*header).cmsg_type != libc::SCM_RIGHTS {
                invalid = true;
            } else {
                // Darwin retains the original cmsg_len after MSG_CTRUNC.
                // Only descriptors actually copied into our buffer are ours.
                let offset = header.cast::<u8>().offset_from(control.as_ptr().cast()) as usize;
                let available = (message.msg_controllen as usize)
                    .min(size_of_val(&control))
                    .saturating_sub(offset);
                let copied = ((*header).cmsg_len as usize).min(available);
                let Some(length) = copied.checked_sub(libc::CMSG_LEN(0) as usize) else {
                    return Err(io::Error::new(ErrorKind::InvalidData, "short router ancillary header"));
                };
                let delivered =
                    std::slice::from_raw_parts(libc::CMSG_DATA(header).cast::<RawFd>(), length / size_of::<RawFd>());
                for &raw in delivered {
                    let descriptor = OwnedFd::from_raw_fd(raw);
                    if libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
                        invalid = true;
                    }
                    fds.push(descriptor);
                }
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
        if invalid || fds.len() > MAX_FDS {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "invalid router ancillary descriptors",
            ));
        }
        Ok(count as usize)
    }
}
#[cfg(test)]
mod tests;

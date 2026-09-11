//! One interface over one framed stream, driven by an application.
use crate::device::FrameDevice;
use crate::frames;
use smoltcp::iface::{Config, Interface, PollResult, SocketSet};
use smoltcp::wire::{HardwareAddress, IpCidr, Ipv4Cidr};
use std::io;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

/// What an application reports after a step, so the loop knows whether to
/// poll the interface again before waiting for the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Nothing moved; wait for packets or a timer.
    Idle,
    /// Sockets were read or written; poll again so the interface acts on it.
    Changed,
    /// The application is finished; flush and return.
    Done,
}

/// Sockets and the logic between them. The stack owns the interface and the
/// socket set; the application owns what happens on the sockets.
pub trait App {
    /// Create sockets, listen or connect. Called once before the first poll.
    fn attach(&mut self, iface: &mut Interface, sockets: &mut SocketSet<'static>);
    /// Move bytes between socket buffers and the application, once.
    fn step(&mut self, sockets: &mut SocketSet<'static>) -> Progress;
}

pub struct Stack<IO> {
    reader: ReadHalf<IO>,
    writer: WriteHalf<IO>,
    device: FrameDevice,
    iface: Interface,
    sockets: SocketSet<'static>,
    started: Instant,
    /// Framed bytes not yet on the wire, and how far into them the writer is.
    outgoing: Vec<u8>,
    written: usize,
}

impl<IO: AsyncRead + AsyncWrite + Unpin> Stack<IO> {
    pub fn new(io: IO, address: Ipv4Cidr, mtu: usize) -> Self {
        let mut device = FrameDevice::new(mtu);
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = seed();
        let started = Instant::now();
        let mut iface = Interface::new(config, &mut device, smoltcp::time::Instant::ZERO);
        iface.update_ip_addrs(|addresses| {
            addresses
                .push(IpCidr::Ipv4(address))
                .expect("a fresh interface holds one address");
        });
        let (reader, writer) = tokio::io::split(io);
        Self {
            reader,
            writer,
            device,
            iface,
            sockets: SocketSet::new(Vec::new()),
            started,
            outgoing: Vec::new(),
            written: 0,
        }
    }

    fn now(&self) -> smoltcp::time::Instant {
        smoltcp::time::Instant::from_micros(i64::try_from(self.started.elapsed().as_micros()).unwrap_or(i64::MAX))
    }

    /// Run until the application is done or the stream ends.
    ///
    /// Returns `Ok(false)` when the peer closed the stream before the
    /// application finished, `Ok(true)` when the application finished.
    pub async fn run(mut self, app: &mut impl App) -> io::Result<bool> {
        app.attach(&mut self.iface, &mut self.sockets);
        let mut packet = Vec::with_capacity(frames::MAX_FRAME_BYTES);
        loop {
            // Poll and step until neither side has anything left to do, so
            // every reply and every buffered write leaves in this round.
            let done = loop {
                let polled = self.iface.poll(self.now(), &mut self.device, &mut self.sockets);
                match app.step(&mut self.sockets) {
                    Progress::Done => {
                        self.iface.poll(self.now(), &mut self.device, &mut self.sockets);
                        break true;
                    }
                    Progress::Changed => continue,
                    Progress::Idle if matches!(polled, PollResult::SocketStateChanged) => continue,
                    Progress::Idle => break false,
                }
            };
            self.frame_outgoing();
            if done {
                return self.drain().await.map(|delivered| delivered && true);
            }
            let timer = self
                .iface
                .poll_delay(self.now(), &self.sockets)
                .map(|delay| std::time::Duration::from_micros(delay.total_micros()));
            let wait = async {
                match timer {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => std::future::pending().await,
                }
            };
            // Read and write at once. Each side of a link can be flooding the
            // other; a loop that writes until its queue is empty before it
            // reads again deadlocks against a peer doing the same, on this
            // pipe as on a VSOCK. `write` is cancel-safe: a read that wins
            // the race leaves the frame boundary where it was.
            let has_outgoing = self.written < self.outgoing.len();
            let has_room = self.device.has_rx_room();
            tokio::select! {
                wrote = self.writer.write(&self.outgoing[self.written..]), if has_outgoing => match wrote {
                    Ok(0) => return Ok(false),
                    Ok(count) => {
                        self.written += count;
                        if self.written == self.outgoing.len() {
                            self.outgoing.clear();
                            self.written = 0;
                        }
                    }
                    Err(error) if peer_gone(&error) => return Ok(false),
                    Err(error) => return Err(error),
                },
                read = frames::read_frame(&mut self.reader, &mut packet), if has_room => match read? {
                    None => return Ok(false),
                    Some(length) => self.device.push_rx(packet[..length].to_vec()),
                },
                () = wait => {}
            }
        }
    }

    /// Move every packet the interface produced into the outgoing bytes.
    fn frame_outgoing(&mut self) {
        while let Some(packet) = self.device.pop_tx() {
            let length = u16::try_from(packet.len()).expect("the device MTU is a u16");
            self.outgoing.extend_from_slice(&length.to_be_bytes());
            self.outgoing.extend_from_slice(&packet);
        }
    }

    /// Write what is left once the application is done. `Ok(false)` when
    /// the peer has gone: a broken pipe or reset is the end of the link,
    /// not a fault in the stack.
    async fn drain(&mut self) -> io::Result<bool> {
        let result = async {
            self.writer.write_all(&self.outgoing[self.written..]).await?;
            self.writer.flush().await
        }
        .await;
        match result {
            Ok(()) => Ok(true),
            Err(error) if peer_gone(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

fn peer_gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
    )
}

/// TCP sequence and port randomisation only need to differ between runs.
fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0)
}

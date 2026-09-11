//! One interface over one framed stream, driven by an application.
use crate::device::FrameDevice;
use crate::frames::FrameParser;
use smoltcp::iface::{Config, Interface, PollResult, SocketSet};
use smoltcp::wire::{HardwareAddress, IpCidr, Ipv4Cidr};
use std::io;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

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

/// Framed bytes waiting for a peer that has stopped reading. Past this the
/// device's transmit queue is left full, which smoltcp reads as a busy link
/// and stops producing segments -- including retransmissions, which would
/// otherwise keep adding copies of the same window for as long as the peer
/// stays stalled.
const OUTGOING_HIGH_WATER_BYTES: usize = 8 * 1024 * 1024;

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
    /// Bytes read but not yet a whole frame.
    incoming: FrameParser,
    /// What crossed the link, reported when it ends: the numbers a stalled
    /// transfer is diagnosed from.
    frames_in: u64,
    bytes_in: u64,
    frames_out: u64,
    bytes_out: u64,
    malformed: u64,
}

/// One read per wakeup; a whole frame at the link MTU fits in two.
const READ_BYTES: usize = 64 * 1024;

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
            incoming: FrameParser::default(),
            frames_in: 0,
            bytes_in: 0,
            frames_out: 0,
            bytes_out: 0,
            malformed: 0,
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
        let mut scratch = vec![0u8; READ_BYTES];
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
            if self.outgoing.len() - self.written < OUTGOING_HIGH_WATER_BYTES {
                self.frame_outgoing();
            }
            if done {
                let delivered = self.drain().await?;
                self.report("application finished");
                return Ok(delivered);
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
                    Ok(0) => {
                        self.report("peer stopped reading");
                        return Ok(false);
                    }
                    Ok(count) => {
                        self.written += count;
                        if self.written == self.outgoing.len() {
                            self.outgoing.clear();
                            self.written = 0;
                        }
                    }
                    Err(error) if peer_gone(&error) => {
                        self.report("peer gone on write");
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                },
                read = self.reader.read(&mut scratch), if has_room => match read {
                    Ok(0) => {
                        self.report("stream ended");
                        return Ok(false);
                    }
                    Ok(count) => self.receive(&scratch[..count])?,
                    Err(error) if peer_gone(&error) => {
                        self.report("peer gone on read");
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                },
                () = wait => {}
            }
        }
    }

    /// Feed bytes to the parser and every complete frame to the device. A
    /// frame that is not IPv4 is counted and dropped, as a NIC drops a
    /// frame it cannot address; smoltcp would drop it later anyway, but the
    /// count is what shows a desynchronised stream.
    fn receive(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.incoming.extend(bytes);
        while let Some(packet) = self
            .incoming
            .next_frame()
            .inspect_err(|_| self.report("corrupt frame"))?
        {
            self.frames_in += 1;
            self.bytes_in += packet.len() as u64;
            if packet.first().map(|byte| byte >> 4) != Some(4) {
                self.malformed += 1;
                continue;
            }
            self.device.push_rx(packet);
        }
        Ok(())
    }

    fn report(&self, outcome: &str) {
        tracing::info!(
            outcome,
            frames_in = self.frames_in,
            bytes_in = self.bytes_in,
            frames_out = self.frames_out,
            bytes_out = self.bytes_out,
            malformed = self.malformed,
            pending_bytes = self.incoming.pending(),
            unsent_bytes = self.outgoing.len() - self.written,
            "network stack link ended"
        );
    }

    /// Move every packet the interface produced into the outgoing bytes.
    fn frame_outgoing(&mut self) {
        while let Some(packet) = self.device.pop_tx() {
            let length = u16::try_from(packet.len()).expect("the device MTU is a u16");
            self.outgoing.extend_from_slice(&length.to_be_bytes());
            self.outgoing.extend_from_slice(&packet);
            self.frames_out += 1;
            self.bytes_out += packet.len() as u64;
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
            Err(error) if peer_gone(&error) => {
                self.report("peer gone on drain");
                Ok(false)
            }
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

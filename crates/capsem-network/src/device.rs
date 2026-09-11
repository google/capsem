//! A smoltcp device whose packets are queues of frames.
//!
//! smoltcp is synchronous: it asks the device for a packet and hands it one
//! to send, both without blocking. The async side of the stack fills the
//! receive queue from the stream between polls and drains the transmit queue
//! after them; the device itself never touches I/O.
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use std::collections::VecDeque;

/// Packets buffered in each direction. Receive is bounded because the poll
/// loop only reads from the stream when there is room; transmit is bounded
/// because smoltcp checks `transmit()` before producing a packet, so a full
/// queue is back-pressure it understands.
pub const QUEUE_PACKETS: usize = 64;

pub struct FrameDevice {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
    mtu: usize,
}

impl FrameDevice {
    pub fn new(mtu: usize) -> Self {
        Self {
            rx: VecDeque::with_capacity(QUEUE_PACKETS),
            tx: VecDeque::with_capacity(QUEUE_PACKETS),
            mtu,
        }
    }

    pub fn has_rx_room(&self) -> bool {
        self.rx.len() < QUEUE_PACKETS
    }

    /// Queue a received packet; a packet larger than the MTU is dropped, as
    /// a NIC would drop an oversized frame.
    pub fn push_rx(&mut self, packet: Vec<u8>) {
        if packet.len() <= self.mtu {
            self.rx.push_back(packet);
        }
    }

    pub fn pop_tx(&mut self) -> Option<Vec<u8>> {
        self.tx.pop_front()
    }
}

impl Device for FrameDevice {
    type RxToken<'a> = Received;
    type TxToken<'a> = Transmit<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let packet = self.rx.pop_front()?;
        Some((Received(packet), Transmit { queue: &mut self.tx }))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        (self.tx.len() < QUEUE_PACKETS).then_some(Transmit { queue: &mut self.tx })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ip;
        capabilities.max_transmission_unit = self.mtu;
        capabilities
    }
}

pub struct Received(Vec<u8>);

impl RxToken for Received {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.0)
    }
}

pub struct Transmit<'a> {
    queue: &'a mut VecDeque<Vec<u8>>,
}

impl TxToken for Transmit<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut packet = vec![0u8; len];
        let result = f(&mut packet);
        self.queue.push_back(packet);
        result
    }
}

//! Minimal 16550 UART emulation for x86_64 port I/O serial console.
//!
//! Handles the standard COM1 port range (0x3F8-0x3FF) with just enough
//! register emulation for kernel boot output:
//! - THR (offset 0): transmit holding register (write -> host pipe)
//! - RBR (offset 0): receive buffer register (read <- host pipe)
//! - LSR (offset 5): line status register (always ready)
//!
//! All other registers (IER, IIR, MCR, MSR, FCR) return 0 / ignore writes.

use std::io::Write;
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use super::pio::PioDevice;

/// 16550 UART register offsets within the 8-byte I/O port range.
const THR: u16 = 0; // Transmit Holding Register (write)
const RBR: u16 = 0; // Receive Buffer Register (read)
const DLL: u16 = 0; // Divisor Latch Low (when DLAB=1)
const LCR: u16 = 3; // Line Control Register
const LSR: u16 = 5; // Line Status Register

/// LCR bits.
const LCR_DLAB: u8 = 0x80; // Divisor Latch Access Bit

/// LSR status bits.
const LSR_DR: u8 = 0x01; // Data Ready (input available)
const LSR_THRE: u8 = 0x20; // Transmitter Holding Register Empty
const LSR_TEMT: u8 = 0x40; // Transmitter Empty

/// Minimal 16550 UART backed by pipe file descriptors.
pub(super) struct Serial16550 {
    tx: Mutex<std::fs::File>,
    // rx_fd for future input support (not used in initial implementation)
    _rx_fd: RawFd,
    lcr: AtomicU8,
}

impl Serial16550 {
    /// Create a new 16550 UART.
    /// - `tx_fd`: write end of the output pipe (guest -> host serial output)
    /// - `rx_fd`: read end of the input pipe (host -> guest serial input)
    pub fn new(tx_fd: RawFd, rx_fd: RawFd) -> Self {
        Self {
            // Safety: tx_fd is a valid pipe fd provided by the caller.
            tx: Mutex::new(unsafe { std::fs::File::from_raw_fd(tx_fd) }),
            _rx_fd: rx_fd,
            lcr: AtomicU8::new(0),
        }
    }
}

impl PioDevice for Serial16550 {
    fn read(&self, port_offset: u16, data: &mut [u8]) {
        if data.is_empty() {
            return;
        }
        match port_offset {
            RBR => {
                // If DLAB=1, this is DLL (Divisor Latch Low), return 0
                // If DLAB=0, this is RBR. No input buffered, return 0
                data[0] = 0;
            }
            LCR => {
                data[0] = self.lcr.load(Ordering::Relaxed);
            }
            LSR => {
                // Always report transmitter ready, no input data
                data[0] = LSR_THRE | LSR_TEMT;
            }
            _ => {
                // All other registers: return 0
                data[0] = 0;
            }
        }
    }

    fn write(&self, port_offset: u16, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        match port_offset {
            THR => {
                let lcr = self.lcr.load(Ordering::Relaxed);
                if lcr & LCR_DLAB != 0 {
                    // DLAB is set, this is DLL (Divisor Latch Low)
                    // We don't care about the baud rate, just ignore it.
                } else {
                    // DLAB is clear, this is THR (Transmit Holding Register)
                    if let Ok(mut tx) = self.tx.lock() {
                        let _ = tx.write_all(&data[..1]);
                    }
                }
            }
            LCR => {
                self.lcr.store(data[0], Ordering::Relaxed);
            }
            _ => {
                // Ignore writes to other registers (IER, FCR, MCR)
            }
        }
    }
}

#[cfg(test)]
mod tests;

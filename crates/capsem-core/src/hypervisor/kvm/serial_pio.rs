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
use std::sync::Mutex;

use super::pio::PioDevice;

/// 16550 UART register offsets within the 8-byte I/O port range.
const THR: u16 = 0; // Transmit Holding Register (write)
const RBR: u16 = 0; // Receive Buffer Register (read)
const LSR: u16 = 5; // Line Status Register

/// LSR status bits.
const LSR_DR: u8 = 0x01;   // Data Ready (input available)
const LSR_THRE: u8 = 0x20; // Transmitter Holding Register Empty
const LSR_TEMT: u8 = 0x40; // Transmitter Empty

/// Minimal 16550 UART backed by pipe file descriptors.
pub(super) struct Serial16550 {
    tx: Mutex<std::fs::File>,
    // rx_fd for future input support (not used in initial implementation)
    _rx_fd: RawFd,
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
                // No input buffered -- return 0
                data[0] = 0;
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
                // Transmit the byte to the host pipe
                if let Ok(mut tx) = self.tx.lock() {
                    let _ = tx.write_all(&data[..1]);
                }
            }
            _ => {
                // Ignore writes to other registers (IER, FCR, LCR, MCR)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pipe() -> (RawFd, RawFd) {
        let mut fds = [0i32; 2];
        let ret = unsafe { libc::pipe(fds.as_mut_ptr()) };
        assert_eq!(ret, 0);
        (fds[0], fds[1]) // (read_end, write_end)
    }

    #[test]
    fn lsr_always_ready() {
        let (rx, tx) = make_pipe();
        let uart = Serial16550::new(tx, rx);
        let mut buf = [0u8; 1];
        uart.read(LSR, &mut buf);
        assert_ne!(buf[0] & LSR_THRE, 0, "THRE should be set");
        assert_ne!(buf[0] & LSR_TEMT, 0, "TEMT should be set");
        // Clean up
        unsafe { libc::close(rx); }
        // tx is owned by Serial16550
    }

    #[test]
    fn thr_writes_to_pipe() {
        let (rx, tx) = make_pipe();
        let uart = Serial16550::new(tx, rx);
        uart.write(THR, &[b'A']);
        uart.write(THR, &[b'B']);

        // Read from the pipe
        let mut buf = [0u8; 2];
        let n = unsafe { libc::read(rx, buf.as_mut_ptr() as *mut libc::c_void, 2) };
        assert_eq!(n, 2);
        assert_eq!(&buf, b"AB");
        unsafe { libc::close(rx); }
    }

    #[test]
    fn rbr_returns_zero_when_empty() {
        let (rx, tx) = make_pipe();
        let uart = Serial16550::new(tx, rx);
        let mut buf = [0xFFu8; 1];
        uart.read(RBR, &mut buf);
        assert_eq!(buf[0], 0);
        unsafe { libc::close(rx); }
    }

    #[test]
    fn unknown_register_returns_zero() {
        let (rx, tx) = make_pipe();
        let uart = Serial16550::new(tx, rx);
        let mut buf = [0xFFu8; 1];
        uart.read(3, &mut buf); // MCR
        assert_eq!(buf[0], 0);
        unsafe { libc::close(rx); }
    }

    #[test]
    fn lsr_no_input_data_ready() {
        let (rx, tx) = make_pipe();
        let uart = Serial16550::new(tx, rx);
        let mut buf = [0u8; 1];
        uart.read(LSR, &mut buf);
        assert_eq!(buf[0] & LSR_DR, 0, "DR should NOT be set when no input");
        unsafe { libc::close(rx); }
    }
}

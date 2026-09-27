//! Minimal 16550 UART emulation for x86_64 port I/O serial console.
//!
//! Handles the standard COM1 port range (0x3F8-0x3FF) with the registers the
//! Linux 8250 driver needs for both of its transmit paths:
//! - THR (offset 0): transmit holding register (write -> host pipe)
//! - RBR (offset 0): receive buffer register (read <- host pipe)
//! - IER (offset 1): interrupt enable register (THRI honoured)
//! - IIR (offset 2): interrupt identification (THRI pending or none)
//! - LSR (offset 5): line status register (always ready)
//!
//! printk polls LSR and writes THR, so kernel output never needed an
//! interrupt. Userspace writes to ttyS0 go through the tty layer, which
//! enables THRI and sends the next character only when the transmit-empty
//! interrupt arrives. Without one, everything a process wrote to
//! /dev/console -- a detached container's output, which `capsem logs`
//! reads -- stalled in the guest's tty buffer. The character is on the host
//! pipe as soon as THR is written, so THR is always empty again and every
//! write with THRI enabled raises IRQ 4 once more.
//!
//! MCR, MSR, FCR and the scratch register read 0 and ignore writes.

use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use super::pio::{PioBus, PioDevice};
use super::sys::VmFd;

/// COM1's legacy ISA I/O base, register window and interrupt line.
const COM1_BASE: u16 = 0x3F8;
const COM1_PORTS: u16 = 8;
const COM1_IRQ: u32 = 4;

/// Put a UART on COM1, its interrupt wired to the guest through an irqfd.
pub(super) fn attach_com1(vm: &VmFd, bus: &PioBus, tx_fd: RawFd, rx_fd: RawFd) -> anyhow::Result<Arc<Serial16550>> {
    let interrupt = super::create_irq_eventfd()?;
    vm.irqfd(interrupt.as_raw_fd(), COM1_IRQ)?;
    let uart = Arc::new(Serial16550::new(tx_fd, rx_fd, interrupt));
    bus.register(COM1_BASE, COM1_PORTS, Arc::clone(&uart) as Arc<dyn PioDevice>)?;
    Ok(uart)
}

/// 16550 UART register offsets within the 8-byte I/O port range.
const THR: u16 = 0; // Transmit Holding Register (write)
const RBR: u16 = 0; // Receive Buffer Register (read)
const IER: u16 = 1; // Interrupt Enable Register (DLM when DLAB=1)
const IIR: u16 = 2; // Interrupt Identification Register (read)
const LCR: u16 = 3; // Line Control Register
const LSR: u16 = 5; // Line Status Register

/// LCR bits.
const LCR_DLAB: u8 = 0x80; // Divisor Latch Access Bit

/// IER bits.
const IER_THRI: u8 = 0x02; // Transmitter Holding Register Empty interrupt

/// IIR values.
const IIR_NO_INTERRUPT: u8 = 0x01;
const IIR_THRI: u8 = 0x02; // Transmitter Holding Register Empty

/// LSR status bits.
#[cfg(test)]
const LSR_DR: u8 = 0x01; // Data Ready (input available)
const LSR_THRE: u8 = 0x20; // Transmitter Holding Register Empty
const LSR_TEMT: u8 = 0x40; // Transmitter Empty

/// Minimal 16550 UART backed by pipe file descriptors.
pub(super) struct Serial16550 {
    tx: Mutex<std::fs::File>,
    // rx_fd for future input support (not used in initial implementation)
    _rx_fd: RawFd,
    lcr: AtomicU8,
    ier: AtomicU8,
    /// A transmit-empty interrupt the guest has not yet acknowledged by
    /// reading IIR or writing THR.
    thri_pending: AtomicBool,
    /// eventfd wired to COM1's GSI through KVM irqfd.
    interrupt: std::fs::File,
}

impl Serial16550 {
    /// Create a new 16550 UART.
    /// - `tx_fd`: write end of the output pipe (guest -> host serial output)
    /// - `rx_fd`: read end of the input pipe (host -> guest serial input)
    /// - `interrupt`: eventfd registered as an irqfd for the UART's line
    pub fn new(tx_fd: RawFd, rx_fd: RawFd, interrupt: OwnedFd) -> Self {
        Self {
            // Safety: tx_fd is a valid pipe fd provided by the caller.
            tx: Mutex::new(unsafe { std::fs::File::from_raw_fd(tx_fd) }),
            _rx_fd: rx_fd,
            lcr: AtomicU8::new(0),
            ier: AtomicU8::new(0),
            thri_pending: AtomicBool::new(false),
            interrupt: std::fs::File::from(interrupt),
        }
    }

    /// Re-arm the transmit interrupt after a checkpoint restore.
    ///
    /// Device registers are not in the checkpoint. A guest restored while
    /// its driver waited for a transmit-empty interrupt would otherwise wait
    /// forever, since the driver only rewrites IER when THRI was clear. One
    /// interrupt costs a driver that was idle nothing: it checks its own
    /// copy of IER and ignores it.
    pub fn resume_after_restore(&self) {
        self.ier.fetch_or(IER_THRI, Ordering::SeqCst);
        self.transmitter_empty();
    }

    /// THR is empty: interrupt the guest if it asked to be told.
    fn transmitter_empty(&self) {
        if self.ier.load(Ordering::SeqCst) & IER_THRI == 0 {
            return;
        }
        self.thri_pending.store(true, Ordering::SeqCst);
        // An edge on the line; KVM latches it until the guest can take it.
        // A full eventfd counter already has one pending, so a failed write
        // loses nothing.
        let _ = (&self.interrupt).write(&1u64.to_ne_bytes());
    }

    fn dlab(&self) -> bool {
        self.lcr.load(Ordering::Relaxed) & LCR_DLAB != 0
    }
}

impl PioDevice for Serial16550 {
    fn read(&self, port_offset: u16, data: &mut [u8]) {
        if data.is_empty() {
            return;
        }
        data[0] = match port_offset {
            // DLL when DLAB=1; otherwise RBR, with no input buffered.
            RBR => 0,
            // DLM when DLAB=1; the divisor is not emulated.
            IER if self.dlab() => 0,
            IER => self.ier.load(Ordering::SeqCst),
            // Reading IIR acknowledges the transmit-empty interrupt it reports.
            IIR if self.thri_pending.swap(false, Ordering::SeqCst) => IIR_THRI,
            IIR => IIR_NO_INTERRUPT,
            LCR => self.lcr.load(Ordering::Relaxed),
            // Always report transmitter ready, no input data.
            LSR => LSR_THRE | LSR_TEMT,
            _ => 0,
        };
    }

    fn write(&self, port_offset: u16, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        match port_offset {
            // DLL: the baud rate is not emulated.
            THR if self.dlab() => {}
            THR => {
                self.thri_pending.store(false, Ordering::SeqCst);
                if let Ok(mut tx) = self.tx.lock() {
                    let _ = tx.write_all(&data[..1]);
                }
                self.transmitter_empty();
            }
            // DLM: the baud rate is not emulated.
            IER if self.dlab() => {}
            IER => {
                self.ier.store(data[0], Ordering::SeqCst);
                self.thri_pending.store(false, Ordering::SeqCst);
                // Enabling THRI with THR empty interrupts at once, as a
                // real 16550 does; the driver's startup probe relies on it.
                self.transmitter_empty();
            }
            LCR => {
                self.lcr.store(data[0], Ordering::Relaxed);
            }
            _ => {
                // Ignore writes to other registers (FCR, MCR, SCR)
            }
        }
    }
}

#[cfg(test)]
mod tests;

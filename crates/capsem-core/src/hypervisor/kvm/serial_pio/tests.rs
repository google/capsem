use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use super::*;

fn make_pipe() -> (RawFd, RawFd) {
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(ret, 0);
    (fds[0], fds[1]) // (read_end, write_end)
}

fn make_eventfd() -> OwnedFd {
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(fd >= 0);
    unsafe { OwnedFd::from_raw_fd(fd) }
}

/// A UART whose interrupt line the test can watch: (uart, pipe read end, irq).
fn make_uart() -> (Serial16550, RawFd, RawFd) {
    let (rx, tx) = make_pipe();
    let irq = make_eventfd();
    let irq_raw = irq.as_raw_fd();
    // The eventfd is owned by the UART, which outlives every use below.
    (Serial16550::new(tx, rx, irq), rx, irq_raw)
}

/// How many times the UART raised its interrupt since the last call.
fn raised(irq: RawFd) -> u64 {
    let mut count = 0u64;
    let n = unsafe { libc::read(irq, (&mut count as *mut u64).cast(), 8) };
    if n == 8 {
        count
    } else {
        0
    }
}

fn read_register(uart: &Serial16550, offset: u16) -> u8 {
    let mut buf = [0xFFu8; 1];
    uart.read(offset, &mut buf);
    buf[0]
}

#[test]
fn lsr_always_ready() {
    let (uart, rx, _) = make_uart();
    let lsr = read_register(&uart, LSR);
    assert_ne!(lsr & LSR_THRE, 0, "THRE should be set");
    assert_ne!(lsr & LSR_TEMT, 0, "TEMT should be set");
    unsafe { libc::close(rx) };
}

#[test]
fn thr_writes_to_pipe() {
    let (uart, rx, _) = make_uart();
    uart.write(THR, b"A");
    uart.write(THR, b"B");

    let mut buf = [0u8; 2];
    let n = unsafe { libc::read(rx, buf.as_mut_ptr() as *mut libc::c_void, 2) };
    assert_eq!(n, 2);
    assert_eq!(&buf, b"AB");
    unsafe { libc::close(rx) };
}

#[test]
fn dlab_prevents_thr_writes() {
    let (uart, rx, _) = make_uart();

    uart.write(LCR, &[LCR_DLAB]);
    // This writes DLL, not THR.
    uart.write(THR, &[0x01]);
    uart.write(LCR, &[0x03]); // 8n1
    uart.write(THR, b"X");

    let mut buf = [0u8; 1];
    let n = unsafe { libc::read(rx, buf.as_mut_ptr() as *mut libc::c_void, 1) };
    assert_eq!(n, 1);
    assert_eq!(&buf, b"X");
    unsafe { libc::close(rx) };
}

#[test]
fn rbr_returns_zero_when_empty() {
    let (uart, rx, _) = make_uart();
    assert_eq!(read_register(&uart, RBR), 0);
    unsafe { libc::close(rx) };
}

#[test]
fn lsr_no_input_data_ready() {
    let (uart, rx, _) = make_uart();
    assert_eq!(
        read_register(&uart, LSR) & LSR_DR,
        0,
        "DR should NOT be set when no input"
    );
    unsafe { libc::close(rx) };
}

/// An idle UART says so. IIR used to read 0 -- "an interrupt is pending" --
/// on every read, which convinced Linux's 8250 driver that transmit
/// interrupts worked, so it waited for one that never came.
#[test]
fn iir_reports_no_interrupt_when_idle() {
    let (uart, rx, irq) = make_uart();
    assert_eq!(read_register(&uart, IIR), IIR_NO_INTERRUPT);
    assert_eq!(raised(irq), 0);
    unsafe { libc::close(rx) };
}

/// The regression: userspace writes to ttyS0 go through the tty layer,
/// which enables THRI and waits for the interrupt. Without one, a detached
/// container's output never reached `capsem logs` on x86_64.
#[test]
fn enabling_the_transmit_interrupt_raises_it_because_thr_is_empty() {
    let (uart, rx, irq) = make_uart();

    uart.write(IER, &[IER_THRI]);

    assert_eq!(raised(irq), 1, "the guest must be interrupted");
    assert_eq!(read_register(&uart, IIR), IIR_THRI);
    // Reading IIR acknowledges a transmit-empty interrupt.
    assert_eq!(read_register(&uart, IIR), IIR_NO_INTERRUPT);
    unsafe { libc::close(rx) };
}

/// The 8250 driver writes one character per interrupt and relies on the
/// next one to continue; the character is on the wire at once, so every
/// write leaves THR empty and interrupts again.
#[test]
fn each_character_written_with_thri_enabled_interrupts_again() {
    let (uart, rx, irq) = make_uart();
    uart.write(IER, &[IER_THRI]);
    raised(irq);
    read_register(&uart, IIR);

    for byte in b"abc" {
        uart.write(THR, &[*byte]);
        assert_eq!(raised(irq), 1);
        assert_eq!(read_register(&uart, IIR), IIR_THRI);
    }

    let mut buf = [0u8; 3];
    let n = unsafe { libc::read(rx, buf.as_mut_ptr() as *mut libc::c_void, 3) };
    assert_eq!(n, 3);
    assert_eq!(&buf, b"abc");
    unsafe { libc::close(rx) };
}

/// The driver disables THRI when its buffer drains; nothing may be
/// reported pending after that, or the console's polled writes would
/// interrupt the guest for every character printk emits.
#[test]
fn disabling_the_transmit_interrupt_clears_it_and_polled_writes_stay_quiet() {
    let (uart, rx, irq) = make_uart();
    uart.write(IER, &[IER_THRI]);
    raised(irq);

    uart.write(IER, &[0]);
    assert_eq!(read_register(&uart, IIR), IIR_NO_INTERRUPT);

    uart.write(THR, b"k");
    assert_eq!(raised(irq), 0, "a polled console write must not interrupt");
    assert_eq!(read_register(&uart, IIR), IIR_NO_INTERRUPT);
    unsafe { libc::close(rx) };
}

/// printk's console path saves IER, writes by polling, and restores it.
#[test]
fn ier_reads_back_what_the_guest_wrote() {
    let (uart, rx, _) = make_uart();
    uart.write(IER, &[IER_THRI | 0x01]);
    assert_eq!(read_register(&uart, IER), IER_THRI | 0x01);
    unsafe { libc::close(rx) };
}

/// With DLAB set, offset 1 is the divisor's high byte, not IER.
#[test]
fn dlab_routes_offset_one_to_the_divisor() {
    let (uart, rx, irq) = make_uart();
    uart.write(LCR, &[LCR_DLAB]);
    uart.write(IER, &[IER_THRI]);
    uart.write(LCR, &[0x03]);

    assert_eq!(read_register(&uart, IER), 0);
    assert_eq!(raised(irq), 0);
    unsafe { libc::close(rx) };
}

/// A checkpoint does not carry UART registers. A guest restored while its
/// driver waited for a transmit interrupt would wait forever, so a restore
/// re-arms THRI and raises it once; a driver that was idle ignores it.
#[test]
fn a_restored_uart_interrupts_once_so_a_waiting_driver_resumes() {
    let (uart, rx, irq) = make_uart();

    uart.resume_after_restore();

    assert_eq!(raised(irq), 1);
    assert_eq!(read_register(&uart, IIR), IIR_THRI);
    unsafe { libc::close(rx) };
}

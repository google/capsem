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
    unsafe {
        libc::close(rx);
    }
    // tx is owned by Serial16550
}

#[test]
fn thr_writes_to_pipe() {
    let (rx, tx) = make_pipe();
    let uart = Serial16550::new(tx, rx);
    uart.write(THR, b"A");
    uart.write(THR, b"B");

    // Read from the pipe
    let mut buf = [0u8; 2];
    let n = unsafe { libc::read(rx, buf.as_mut_ptr() as *mut libc::c_void, 2) };
    assert_eq!(n, 2);
    assert_eq!(&buf, b"AB");
    unsafe {
        libc::close(rx);
    }
}

#[test]
fn dlab_prevents_thr_writes() {
    let (rx, tx) = make_pipe();
    let uart = Serial16550::new(tx, rx);

    // Enable DLAB
    uart.write(LCR, &[LCR_DLAB]);

    // This should write to DLL, not THR
    uart.write(THR, &[0x01]);

    // Disable DLAB
    uart.write(LCR, &[0x03]); // 8n1

    // This should write to THR
    uart.write(THR, b"X");

    // Check that only 'X' was written
    let mut buf = [0u8; 1];
    let n = unsafe { libc::read(rx, buf.as_mut_ptr() as *mut libc::c_void, 1) };
    assert_eq!(n, 1);
    assert_eq!(&buf, b"X");

    unsafe {
        libc::close(rx);
    }
}

#[test]
fn rbr_returns_zero_when_empty() {
    let (rx, tx) = make_pipe();
    let uart = Serial16550::new(tx, rx);
    let mut buf = [0xFFu8; 1];
    uart.read(RBR, &mut buf);
    assert_eq!(buf[0], 0);
    unsafe {
        libc::close(rx);
    }
}

#[test]
fn unknown_register_returns_zero() {
    let (rx, tx) = make_pipe();
    let uart = Serial16550::new(tx, rx);
    let mut buf = [0xFFu8; 1];
    uart.read(2, &mut buf); // FCR / IIR
    assert_eq!(buf[0], 0);
    unsafe {
        libc::close(rx);
    }
}

#[test]
fn lsr_no_input_data_ready() {
    let (rx, tx) = make_pipe();
    let uart = Serial16550::new(tx, rx);
    let mut buf = [0u8; 1];
    uart.read(LSR, &mut buf);
    assert_eq!(buf[0] & LSR_DR, 0, "DR should NOT be set when no input");
    unsafe {
        libc::close(rx);
    }
}

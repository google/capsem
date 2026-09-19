use super::*;

#[test]
fn stdin_data_and_explicit_eof_round_trip_as_bounded_big_endian_frames() {
    let mut wire = Vec::new();
    write_exec_input(&mut wire, &ExecInputFrame::Data(vec![0, 255, 10])).unwrap();
    write_exec_input(&mut wire, &ExecInputFrame::StdinEof).unwrap();
    let first_len = u32::from_be_bytes(wire[..4].try_into().unwrap()) as usize;
    assert!(first_len < 64, "byte strings must stay compact: {first_len}");
    let mut reader = wire.as_slice();
    assert_eq!(
        read_exec_input(&mut reader).unwrap(),
        ExecInputFrame::Data(vec![0, 255, 10])
    );
    assert_eq!(read_exec_input(&mut reader).unwrap(), ExecInputFrame::StdinEof);
    assert!(reader.is_empty());
}

#[test]
fn stdout_and_stderr_remain_distinct() {
    for channel in [ExecOutputChannel::Stdout, ExecOutputChannel::Stderr] {
        let expected = ExecOutputFrame {
            channel,
            data: vec![0, 255, 10],
        };
        let mut wire = Vec::new();
        write_exec_output(&mut wire, &expected).unwrap();
        assert_eq!(read_exec_output(&mut wire.as_slice()).unwrap(), expected);
    }
}

#[test]
fn declared_and_actual_oversized_frames_are_rejected() {
    let declared_bytes = (MAX_EXEC_FRAME_BYTES + 1).to_be_bytes();
    let mut declared = declared_bytes.as_slice();
    assert_eq!(
        read_exec_input(&mut declared).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let oversized = ExecInputFrame::Data(vec![0; MAX_EXEC_DATA_BYTES + 1]);
    assert_eq!(
        write_exec_input(&mut Vec::new(), &oversized).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

/// Each `write` on the guest's vsock socket is a syscall.
#[derive(Default)]
struct CountedWrites {
    bytes: Vec<u8>,
    calls: usize,
}

impl Write for CountedWrites {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Finding 31: the header and the payload went out as two writes per frame.
#[test]
fn one_output_frame_leaves_in_one_write() {
    let mut writer = CountedWrites::default();
    let frame = ExecOutputFrame {
        channel: ExecOutputChannel::Stdout,
        data: b"chunk".to_vec(),
    };
    write_exec_output(&mut writer, &frame).unwrap();
    assert_eq!(writer.calls, 1, "header and payload leave together");
    assert_eq!(read_exec_output(&mut writer.bytes.as_slice()).unwrap(), frame);
}

/// The guest frames the bytes it just read without first copying them into an
/// owned frame, and the wire stays exactly what the owned frame encodes to.
#[test]
fn borrowed_output_data_encodes_exactly_like_an_owned_frame() {
    for channel in [ExecOutputChannel::Stdout, ExecOutputChannel::Stderr] {
        let data = [0_u8, 255, 10, 13];
        let mut owned = Vec::new();
        write_exec_output(
            &mut owned,
            &ExecOutputFrame {
                channel,
                data: data.to_vec(),
            },
        )
        .unwrap();
        let mut borrowed = Vec::new();
        write_exec_output_data(&mut borrowed, channel, &data).unwrap();
        assert_eq!(borrowed, owned);
    }
}

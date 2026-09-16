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

use super::*;

#[test]
fn snapshot_freeze_and_thaw_requests_roundtrip_over_agent_control_io() {
    for frozen in [true, false] {
        let (read_fd, write_fd) = make_pipe();
        let msg = HostToGuest::SnapshotTransition { id: 41, frozen };
        write_all_fd(write_fd, &capsem_proto::encode_host_msg(&msg).unwrap()).unwrap();
        let decoded = recv_host_msg(read_fd).unwrap();
        assert!(matches!(decoded, HostToGuest::SnapshotTransition { id: 41, frozen: actual } if actual == frozen));
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }
}

#[test]
fn snapshot_transition_result_roundtrips_over_agent_control_io() {
    for error in [None, Some("still frozen".to_owned())] {
        let (read_fd, write_fd) = make_pipe();
        send_guest_msg(
            write_fd,
            &GuestToHost::SnapshotTransitioned {
                id: 41,
                frozen: false,
                error: error.clone(),
            },
        )
        .unwrap();
        let mut len_buf = [0u8; 4];
        read_exact_fd(read_fd, &mut len_buf).unwrap();
        let mut payload = vec![0u8; u32::from_be_bytes(len_buf) as usize];
        read_exact_fd(read_fd, &mut payload).unwrap();
        let decoded = capsem_proto::decode_guest_msg(&payload).unwrap();
        assert!(
            matches!(decoded, GuestToHost::SnapshotTransitioned { id: 41, frozen: false, error: actual } if actual == error)
        );
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }
}

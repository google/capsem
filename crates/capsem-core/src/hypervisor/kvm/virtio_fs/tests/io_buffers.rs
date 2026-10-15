use super::*;

#[test]
fn read_reuses_bounded_storage_without_leaking_previous_payloads() {
    let dir = temp_share("read-buffer-reuse");
    let bytes = vec![0xA5; MAX_READ_SIZE as usize];
    std::fs::write(dir.join("data"), &bytes).unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "data").unwrap();
    let fh = open_file(&mut proc, ino, 0).unwrap();
    let mut response = Vec::with_capacity(bytes.len() + OUT_HDR_SIZE);
    let allocation = response.as_ptr();
    for (offset, size, expected) in [
        (0, MAX_READ_SIZE, bytes.as_slice()),
        (u64::from(MAX_READ_SIZE) - 3, 512, &bytes[..3]),
        (u64::from(MAX_READ_SIZE), 512, &[]),
    ] {
        let input = FuseReadIn {
            fh,
            offset,
            size,
            read_flags: 0,
            lock_owner: 0,
            flags: 0,
            padding: 0,
        };
        proc.handle_request_into(
            &build_request(&make_header(FUSE_READ, ino, 42), fuse::as_bytes(&input)),
            &mut response,
        );
        let header: FuseOutHeader = fuse::read_struct(&response).unwrap();
        assert_eq!(header.error, 0);
        assert_eq!(header.unique, 42);
        assert_eq!(header.len as usize, response.len());
        assert_eq!(&response[OUT_HDR_SIZE..], expected);
        assert_eq!(
            response.as_ptr(),
            allocation,
            "large/short/EOF reads must reuse one allocation"
        );
    }
    proc.handle_request_into(&build_request(&make_header(FUSE_READ, ino, 43), &[]), &mut response);
    assert_eq!(
        response.len(),
        OUT_HDR_SIZE,
        "errors must not expose the previous payload"
    );
    assert_eq!(response.as_ptr(), allocation);
}

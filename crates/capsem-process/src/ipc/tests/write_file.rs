use super::*;

/// A host-to-guest file write is recorded before it is dispatched, and never
/// dispatched when the ledger refuses its record: a write the session cannot
/// account for does not happen. It used to be written first and recorded
/// after, through a blocking send on a runtime worker whose result was
/// ignored, so a refused record still left the file in the guest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_whose_record_is_refused_never_reaches_the_guest() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, mut ctrl_rx) = Dispatcher::new(temp.path());
    let db = Arc::clone(&dispatcher.net_state.db);
    tokio::task::spawn_blocking(move || db.shutdown_blocking())
        .await
        .unwrap();
    let (process_stream, service_stream) = tokio::net::UnixStream::pair().unwrap();
    let _handler = dispatcher.connection(process_stream);
    let mut service_stream = service_stream.into_std().unwrap();
    let (service_tx, service_rx): (Sender<ServiceToProcess>, Receiver<ProcessToService>) =
        tokio::task::spawn_blocking(move || {
            capsem_foundation::ipc_handshake::negotiate_initiator(&mut service_stream, "capsem-service-test", "")
                .unwrap();
            channel_from_std(service_stream).unwrap()
        })
        .await
        .unwrap();

    service_tx
        .send(ServiceToProcess::WriteFile {
            id: 31,
            path: "/root/unrecorded.txt".to_string(),
            data: b"payload".to_vec(),
        })
        .await
        .unwrap();
    match service_rx.recv().await.unwrap() {
        ProcessToService::WriteFileResult {
            id: 31,
            success: false,
            error: Some(error),
        } => assert!(error.contains("could not be recorded"), "{error}"),
        other => panic!("expected a refused write, got {other:?}"),
    }
    assert!(
        ctrl_rx.try_recv().is_err(),
        "a write the ledger refused must never be dispatched to the guest"
    );
}

#[test]
fn prepared_write_owns_final_frame_and_bounded_audit_facts() {
    let prepared = crate::control_queue::PreparedWrite::new(61, "/workspace/one".into(), vec![23; 128 * 1024]).unwrap();
    let retry = prepared.clone();
    assert_eq!(prepared.frame.bytes().as_ptr(), retry.frame.bytes().as_ptr());
    assert_eq!(prepared.observation.size, 128 * 1024);
    assert_eq!(
        prepared.observation.content.len(),
        super::super::file_boundary::FILE_SECURITY_CONTENT_PREVIEW_MAX
    );
    assert!(
        matches!(capsem_proto::decode_host_msg(&prepared.frame.bytes()[4..]).unwrap(), HostToGuest::FileWrite { id: 61, data, .. } if data == vec![23; 128 * 1024])
    );
}

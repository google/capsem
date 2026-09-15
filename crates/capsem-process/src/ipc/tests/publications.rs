use super::*;

/// The service asks the VM owner, not its own memory, which ports are
/// published: listing reports the owner's generation, and revoking an unknown
/// port answers `revoked: false` instead of an error or silence.
#[tokio::test]
async fn owner_answers_publication_list_and_revoke_requests() {
    let temp = tempfile::tempdir().unwrap();
    let (dispatcher, _ctrl_rx) = Dispatcher::new(temp.path());
    let generation = dispatcher.job_store.publisher.generation().get();
    let (process_stream, service_stream) = tokio::net::UnixStream::pair().unwrap();
    let _handler = dispatcher.connection(process_stream);
    let mut service_stream = service_stream.into_std().unwrap();
    let (service_tx, service_rx) = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service_stream, "capsem-service-test", "").unwrap();
        channel_from_std::<ServiceToProcess, ProcessToService>(service_stream).unwrap()
    })
    .await
    .unwrap();

    service_tx
        .send(ServiceToProcess::ListPublications { id: 71 })
        .await
        .unwrap();
    match service_rx.recv().await.unwrap() {
        ProcessToService::PublicationList {
            id,
            generation: reported,
            publications,
        } => {
            assert_eq!((id, reported), (71, generation));
            assert!(publications.is_empty());
        }
        other => panic!("unexpected list response: {other:?}"),
    }
    service_tx
        .send(ServiceToProcess::RevokePort {
            id: 72,
            host_port: 16379,
        })
        .await
        .unwrap();
    match service_rx.recv().await.unwrap() {
        ProcessToService::PortRevoked { id, revoked, error } => {
            assert_eq!((id, revoked, error), (72, false, None));
        }
        other => panic!("unexpected revoke response: {other:?}"),
    }
}

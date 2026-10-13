use super::*;

#[test]
fn benchmark_limits_are_the_protocol_authority() {
    let limits = report();
    assert_eq!(limits.network_client_capacity, capsem_proto::NETWORK_CLIENT_CAPACITY);
    assert_eq!(
        limits.ledger_producer_queue_capacity,
        capsem_proto::LEDGER_PRODUCER_QUEUE_CAPACITY
    );
    assert_eq!(
        limits.ledger_admission_batch_capacity,
        capsem_proto::LEDGER_ADMISSION_BATCH_CAPACITY
    );
    assert_eq!(
        limits.ledger_admission_coalesce_wait_microseconds,
        capsem_proto::LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS
    );
    assert_eq!(
        limits.ledger_disk_flush_interval_milliseconds,
        capsem_proto::LEDGER_DISK_FLUSH_INTERVAL_MILLISECONDS
    );
    assert_eq!(
        limits.ledger_disk_flush_max_pending_events,
        capsem_proto::LEDGER_DISK_FLUSH_MAX_PENDING_EVENTS
    );
}

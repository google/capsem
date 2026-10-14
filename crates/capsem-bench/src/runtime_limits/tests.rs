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
    assert_eq!(limits.cache_dns_answer_entries, capsem_proto::CACHE_DNS_ANSWER_ENTRIES);
    assert_eq!(
        limits.cache_dns_max_ttl_seconds,
        capsem_proto::CACHE_DNS_MAX_TTL_SECONDS
    );
    assert_eq!(
        limits.cache_dns_negative_ttl_seconds,
        capsem_proto::CACHE_DNS_NEGATIVE_TTL_SECONDS
    );
    assert_eq!(limits.cache_tls_leaf_entries, capsem_proto::CACHE_TLS_LEAF_ENTRIES);
    assert_eq!(
        limits.cache_ledger_query_batch_entries,
        capsem_proto::CACHE_LEDGER_QUERY_BATCH_ENTRIES
    );
    assert_eq!(
        limits.cache_sqlite_prepared_statements,
        capsem_proto::CACHE_SQLITE_PREPARED_STATEMENTS
    );
    assert_eq!(
        limits.cache_service_storage_diagnostics_entries,
        capsem_proto::CACHE_SERVICE_STORAGE_DIAGNOSTICS_ENTRIES
    );
    assert_eq!(
        limits.cache_service_persistent_resume_entries,
        capsem_proto::CACHE_SERVICE_PERSISTENT_RESUME_ENTRIES
    );
    assert_eq!(
        limits.cache_update_check_ttl_seconds,
        capsem_proto::CACHE_UPDATE_CHECK_TTL_SECONDS
    );
    assert_eq!(
        limits.cache_image_catalog_refresh_seconds,
        capsem_proto::CACHE_IMAGE_CATALOG_REFRESH_SECONDS
    );
    assert_eq!(
        limits.cache_image_catalog_retry_seconds,
        capsem_proto::CACHE_IMAGE_CATALOG_RETRY_SECONDS
    );
    assert_eq!(
        limits.cache_image_verification_batch_keys,
        capsem_proto::CACHE_IMAGE_VERIFICATION_BATCH_KEYS
    );
}

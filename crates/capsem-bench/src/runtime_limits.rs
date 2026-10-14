//! Config-owned capacities exposed to the benchmark curve planner.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct Report {
    network_client_capacity: usize,
    network_dns_sessions_per_vm: usize,
    network_dns_max_udp_datagram_bytes: usize,
    network_dns_udp_socket_buffer_bytes: usize,
    ledger_producer_queue_capacity: usize,
    ledger_admission_batch_capacity: usize,
    ledger_admission_coalesce_wait_microseconds: u64,
    ledger_sqlite_mmap_bytes: i64,
    ledger_disk_flush_interval_milliseconds: u64,
    ledger_disk_flush_max_pending_events: usize,
    cache_dns_answer_entries: usize,
    cache_dns_max_ttl_seconds: u32,
    cache_dns_negative_ttl_seconds: u32,
    cache_tls_leaf_entries: usize,
    cache_ledger_query_batch_entries: usize,
    cache_sqlite_prepared_statements: usize,
    cache_service_storage_diagnostics_entries: usize,
    cache_service_persistent_resume_entries: usize,
    cache_update_check_ttl_seconds: u64,
    cache_image_catalog_refresh_seconds: u64,
    cache_image_catalog_retry_seconds: u64,
    cache_image_verification_batch_keys: usize,
}

pub(crate) const fn report() -> Report {
    Report {
        network_client_capacity: capsem_proto::NETWORK_CLIENT_CAPACITY,
        network_dns_sessions_per_vm: capsem_proto::NETWORK_DNS_SESSIONS_PER_VM,
        network_dns_max_udp_datagram_bytes: capsem_proto::NETWORK_DNS_MAX_UDP_DATAGRAM_BYTES,
        network_dns_udp_socket_buffer_bytes: capsem_proto::NETWORK_DNS_UDP_SOCKET_BUFFER_BYTES,
        ledger_producer_queue_capacity: capsem_proto::LEDGER_PRODUCER_QUEUE_CAPACITY,
        ledger_admission_batch_capacity: capsem_proto::LEDGER_ADMISSION_BATCH_CAPACITY,
        ledger_admission_coalesce_wait_microseconds: capsem_proto::LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS,
        ledger_sqlite_mmap_bytes: capsem_proto::LEDGER_SQLITE_MMAP_BYTES,
        ledger_disk_flush_interval_milliseconds: capsem_proto::LEDGER_DISK_FLUSH_INTERVAL_MILLISECONDS,
        ledger_disk_flush_max_pending_events: capsem_proto::LEDGER_DISK_FLUSH_MAX_PENDING_EVENTS,
        cache_dns_answer_entries: capsem_proto::CACHE_DNS_ANSWER_ENTRIES,
        cache_dns_max_ttl_seconds: capsem_proto::CACHE_DNS_MAX_TTL_SECONDS,
        cache_dns_negative_ttl_seconds: capsem_proto::CACHE_DNS_NEGATIVE_TTL_SECONDS,
        cache_tls_leaf_entries: capsem_proto::CACHE_TLS_LEAF_ENTRIES,
        cache_ledger_query_batch_entries: capsem_proto::CACHE_LEDGER_QUERY_BATCH_ENTRIES,
        cache_sqlite_prepared_statements: capsem_proto::CACHE_SQLITE_PREPARED_STATEMENTS,
        cache_service_storage_diagnostics_entries: capsem_proto::CACHE_SERVICE_STORAGE_DIAGNOSTICS_ENTRIES,
        cache_service_persistent_resume_entries: capsem_proto::CACHE_SERVICE_PERSISTENT_RESUME_ENTRIES,
        cache_update_check_ttl_seconds: capsem_proto::CACHE_UPDATE_CHECK_TTL_SECONDS,
        cache_image_catalog_refresh_seconds: capsem_proto::CACHE_IMAGE_CATALOG_REFRESH_SECONDS,
        cache_image_catalog_retry_seconds: capsem_proto::CACHE_IMAGE_CATALOG_RETRY_SECONDS,
        cache_image_verification_batch_keys: capsem_proto::CACHE_IMAGE_VERIFICATION_BATCH_KEYS,
    }
}

#[cfg(test)]
mod tests;

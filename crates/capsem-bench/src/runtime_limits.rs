//! Config-owned capacities exposed to the benchmark curve planner.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct Report {
    network_client_capacity: usize,
    ledger_producer_queue_capacity: usize,
    ledger_admission_batch_capacity: usize,
    ledger_admission_coalesce_wait_microseconds: u64,
    ledger_disk_flush_interval_milliseconds: u64,
    ledger_disk_flush_max_pending_events: usize,
}

pub(crate) const fn report() -> Report {
    Report {
        network_client_capacity: capsem_proto::NETWORK_CLIENT_CAPACITY,
        ledger_producer_queue_capacity: capsem_proto::LEDGER_PRODUCER_QUEUE_CAPACITY,
        ledger_admission_batch_capacity: capsem_proto::LEDGER_ADMISSION_BATCH_CAPACITY,
        ledger_admission_coalesce_wait_microseconds: capsem_proto::LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS,
        ledger_disk_flush_interval_milliseconds: capsem_proto::LEDGER_DISK_FLUSH_INTERVAL_MILLISECONDS,
        ledger_disk_flush_max_pending_events: capsem_proto::LEDGER_DISK_FLUSH_MAX_PENDING_EVENTS,
    }
}

#[cfg(test)]
mod tests;

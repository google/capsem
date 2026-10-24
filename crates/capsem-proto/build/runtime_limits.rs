use std::fmt::Write;
use std::path::PathBuf;

pub fn write_runtime_limits() {
    // Cargo packages the canonical config symlink as a regular file, so
    // registry consumers generate the same constants without our checkout.
    let config_path = PathBuf::from("runtime-limits.toml");
    let source = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("runtime limits: read {}: {error}", config_path.display()));
    let config: toml::Value = toml::from_str(&source)
        .unwrap_or_else(|error| panic!("runtime limits: parse {}: {error}", config_path.display()));
    let capacity = config
        .get("network")
        .and_then(|network| network.get("max_concurrent_clients"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (1..=4096).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: network.max_concurrent_clients must be an integer between 1 and 4096")
        });
    let traffic_grants_per_client = config
        .get("network")
        .and_then(|network| network.get("traffic_grants_per_client"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (3..=4).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: network.traffic_grants_per_client must be an integer between 3 and 4")
        });
    let descriptors_per_client = config
        .get("network")
        .and_then(|network| network.get("file_descriptors_per_client"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (3..=64).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: network.file_descriptors_per_client must be an integer between 3 and 64")
        });
    let traffic_grant_capacity = capacity
        .checked_mul(traffic_grants_per_client)
        .filter(|value| *value <= 16_384)
        .expect("runtime limits: derived network traffic-grant capacity exceeds 16384");
    if descriptors_per_client < traffic_grants_per_client * 2 {
        panic!("runtime limits: network.file_descriptors_per_client must cover both ends of every traffic grant");
    }
    let open_file_limit = capacity
        .checked_mul(descriptors_per_client)
        .filter(|value| *value <= 1_048_576)
        .expect("runtime limits: derived network descriptor budget exceeds 1048576");
    let dns_sessions_per_vm = config
        .get("network")
        .and_then(|network| network.get("dns_sessions_per_vm"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (2..=8).contains(value))
        .unwrap_or_else(|| panic!("runtime limits: network.dns_sessions_per_vm must be an integer between 2 and 8"));
    let dns_max_udp_datagram_bytes = config
        .get("network")
        .and_then(|network| network.get("dns_max_udp_datagram_bytes"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (4096..=65_507).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: network.dns_max_udp_datagram_bytes must be an integer between 4096 and 65507")
        });
    let dns_udp_socket_buffer_bytes = capacity
        .checked_mul(dns_sessions_per_vm)
        .and_then(|value| value.checked_mul(dns_max_udp_datagram_bytes))
        .filter(|value| *value <= 64 * 1024 * 1024)
        .expect("runtime limits: derived DNS UDP socket buffer exceeds 64 MiB");
    let producer_events_per_network_client = config
        .get("ledger")
        .and_then(|ledger| ledger.get("producer_events_per_network_client"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (1..=16).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: ledger.producer_events_per_network_client must be an integer between 1 and 16")
        });
    let ledger_queue_capacity = capacity
        .checked_mul(producer_events_per_network_client)
        .filter(|value| *value <= 65_536)
        .expect("runtime limits: derived ledger producer queue exceeds 65536");
    let ledger_batch_capacity = config
        .get("ledger")
        .and_then(|ledger| ledger.get("admission_batch_capacity"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (1..=1024).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: ledger.admission_batch_capacity must be an integer between 1 and 1024")
        });
    if ledger_batch_capacity > ledger_queue_capacity || ledger_queue_capacity % ledger_batch_capacity != 0 {
        panic!("runtime limits: ledger producer queue must be an exact multiple of admission batch capacity");
    }
    let ledger_coalesce_wait_microseconds = config
        .get("ledger")
        .and_then(|ledger| ledger.get("admission_coalesce_wait_microseconds"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (1..=10_000).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: ledger.admission_coalesce_wait_microseconds must be an integer between 1 and 10000")
        });
    let ledger_archive_index_rows_per_flush = config
        .get("ledger")
        .and_then(|ledger| ledger.get("archive_index_rows_per_flush"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (ledger_batch_capacity..=65_536).contains(value))
        .filter(|value| value % ledger_batch_capacity == 0)
        .unwrap_or_else(|| {
            panic!("runtime limits: ledger.archive_index_rows_per_flush must be a batch multiple between the batch capacity and 65536")
        });
    let ledger_sqlite_mmap_bytes = config
        .get("ledger")
        .and_then(|ledger| ledger.get("sqlite_mmap_bytes"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (4 * 1024 * 1024..=256 * 1024 * 1024).contains(value))
        .filter(|value| value % (1024 * 1024) == 0)
        .unwrap_or_else(|| {
            panic!("runtime limits: ledger.sqlite_mmap_bytes must be a whole MiB between 4 MiB and 256 MiB")
        });
    let ledger_disk_flush_interval_milliseconds = config
        .get("ledger")
        .and_then(|ledger| ledger.get("disk_flush_interval_milliseconds"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (100..=5_000).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: ledger.disk_flush_interval_milliseconds must be an integer between 100 and 5000")
        });
    let ledger_disk_flush_max_pending_events = config
        .get("ledger")
        .and_then(|ledger| ledger.get("disk_flush_max_pending_events"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (ledger_batch_capacity..=10_000_000).contains(value))
        .filter(|value| value % ledger_batch_capacity == 0)
        .unwrap_or_else(|| {
            panic!("runtime limits: ledger.disk_flush_max_pending_events must be a batch multiple between the batch capacity and 10000000")
        });
    let cache = config
        .get("cache")
        .and_then(toml::Value::as_table)
        .unwrap_or_else(|| panic!("runtime limits: cache must be a table"));
    let cache_integer = |name: &str, minimum: i64, maximum: i64| {
        cache
            .get(name)
            .and_then(toml::Value::as_integer)
            .filter(|value| (minimum..=maximum).contains(value))
            .unwrap_or_else(|| {
                panic!("runtime limits: cache.{name} must be an integer between {minimum} and {maximum}")
            })
    };
    let cache_dns_answer_entries = cache_integer("dns_answer_entries", 1, 65_536);
    let cache_dns_max_ttl_seconds = cache_integer("dns_max_ttl_seconds", 1, 86_400);
    let cache_dns_negative_ttl_seconds = cache_integer("dns_negative_ttl_seconds", 1, cache_dns_max_ttl_seconds);
    let cache_tls_leaf_entries = cache_integer("tls_leaf_entries", 1, 65_536);
    let cache_ledger_query_batch_entries = cache_integer("ledger_query_batch_entries", 1, 65_536);
    let cache_sqlite_prepared_statements = cache_integer("sqlite_prepared_statements", 1, 65_536);
    let cache_service_storage_diagnostics_entries = cache_integer("service_storage_diagnostics_entries", 1, 65_536);
    let cache_service_persistent_resume_entries = cache_integer("service_persistent_resume_entries", 1, 65_536);
    let cache_update_check_ttl_seconds = cache_integer("update_check_ttl_seconds", 60, 604_800);
    let cache_image_catalog_refresh_seconds = cache_integer("image_catalog_refresh_seconds", 60, 86_400);
    let cache_image_catalog_retry_seconds =
        cache_integer("image_catalog_retry_seconds", 1, cache_image_catalog_refresh_seconds);
    let cache_image_verification_batch_keys = cache_integer("image_verification_batch_keys", 1, 65_536);
    let mcp_integer = |key: &str, maximum: i64| {
        config
            .get("mcp")
            .and_then(|mcp| mcp.get(key))
            .and_then(toml::Value::as_integer)
            .filter(|value| (1..=maximum).contains(value))
            .unwrap_or_else(|| panic!("runtime limits: mcp.{key} must be between 1 and {maximum}"))
    };
    let mcp_http_timeout_seconds = mcp_integer("http_timeout_seconds", 3600);
    let mcp_startup_timeout_seconds = mcp_integer("startup_timeout_seconds", 300);
    let mcp_worker_queue_capacity = mcp_integer("worker_queue_capacity", 65_536);
    let mcp_builtin_peers_per_session = mcp_integer("builtin_peers_per_session", 16);
    let terminal_limits = config["terminal"]
        .as_table()
        .expect("terminal limits table")
        .iter()
        .fold(String::new(), |mut output, (key, value)| {
            let value = value
                .as_integer()
                .filter(|v| (1..=1_048_576).contains(v))
                .expect("positive bounded terminal limit");
            writeln!(
                output,
                "pub const TERMINAL_{}: usize = {value};",
                key.to_ascii_uppercase()
            )
            .expect("write terminal limit");
            output
        });
    let approval_limits = super::approval_limits::render(&config);
    let update_limits = super::update_limits::render(&config);
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    std::fs::write(
        format!("{out_dir}/runtime_limits.rs"),
        format!(
            "/// Supported concurrent network-client ceiling for one VM.\n\
             ///\n\
             /// TCP descriptor grants and each of the two DNS sessions derive\n\
             /// their bounds from this one config-owned value.\n\
             pub const NETWORK_CLIENT_CAPACITY: usize = {capacity};\n\
             /// Native MCP HTTP response-header and body request deadline.\n\
             pub const MCP_HTTP_TIMEOUT_SECONDS: u64 = {mcp_http_timeout_seconds};\n\
             /// Native MCP bootstrap/readiness deadline.\n\
             pub const MCP_STARTUP_TIMEOUT_SECONDS: u64 = {mcp_startup_timeout_seconds};\n\
             /// Bounded native MCP request/response queue capacity.\n\
             pub const MCP_WORKER_QUEUE_CAPACITY: usize = {mcp_worker_queue_capacity};\n\
             /// Approved first-party tool peers for one VM generation.\n\
             pub const MCP_BUILTIN_PEERS_PER_SESSION: usize = {mcp_builtin_peers_per_session};\n\
             /// Live and retiring traffic grants allowed during connection replacement.\n\
             pub const NETWORK_TRAFFIC_GRANT_CAPACITY: usize = {traffic_grant_capacity};\n\
             /// Minimum descriptor budget for a network-facing process.\n\
             pub const NETWORK_PROCESS_OPEN_FILE_LIMIT: u64 = {open_file_limit};\n\
             /// Independent guest-to-host DNS sessions per VM.\n\
             pub const NETWORK_DNS_SESSIONS_PER_VM: usize = {dns_sessions_per_vm};\n\
             /// Maximum DNS datagram accepted by the guest proxy.\n\
             pub const NETWORK_DNS_MAX_UDP_DATAGRAM_BYTES: usize = {dns_max_udp_datagram_bytes};\n\
             /// Receive buffer for one full supported DNS client burst.\n\
             pub const NETWORK_DNS_UDP_SOCKET_BUFFER_BYTES: usize = {dns_udp_socket_buffer_bytes};\n\
             /// Per-producer security-event queue derived from supported clients.\n\
             pub const LEDGER_PRODUCER_QUEUE_CAPACITY: usize = {ledger_queue_capacity};\n\
             /// Consecutive producer events coalesced into one ledger exchange.\n\
             pub const LEDGER_ADMISSION_BATCH_CAPACITY: usize = {ledger_batch_capacity};\n\
             /// Maximum wait for an immediate follower before admitting a singleton.\n\
             pub const LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS: u64 = {ledger_coalesce_wait_microseconds};\n\
             /// Archive body-index rows published in one disk flush transaction.\n\
             pub const LEDGER_ARCHIVE_INDEX_ROWS_PER_FLUSH: usize = {ledger_archive_index_rows_per_flush};\n\
             /// Maximum file-backed ledger window mapped into one process.\n\
             pub const LEDGER_SQLITE_MMAP_BYTES: i64 = {ledger_sqlite_mmap_bytes};\n\
             /// Maximum time accepted rows stay only in the memory schema.\n\
             pub const LEDGER_DISK_FLUSH_INTERVAL_MILLISECONDS: u64 = {ledger_disk_flush_interval_milliseconds};\n\
             /// Pending events that force a disk flush before the timer.\n\
             pub const LEDGER_DISK_FLUSH_MAX_PENDING_EVENTS: usize = {ledger_disk_flush_max_pending_events};\n\
             /// DNS answers retained per session.\n\
             pub const CACHE_DNS_ANSWER_ENTRIES: usize = {cache_dns_answer_entries};\n\
             /// Longest positive DNS answer reuse.\n\
             pub const CACHE_DNS_MAX_TTL_SECONDS: u32 = {cache_dns_max_ttl_seconds};\n\
             /// Longest negative DNS answer reuse.\n\
             pub const CACHE_DNS_NEGATIVE_TTL_SECONDS: u32 = {cache_dns_negative_ttl_seconds};\n\
             /// Minted TLS leaves retained per proxy.\n\
             pub const CACHE_TLS_LEAF_ENTRIES: usize = {cache_tls_leaf_entries};\n\
             /// Distinct ledger query batches retained per reader.\n\
             pub const CACHE_LEDGER_QUERY_BATCH_ENTRIES: usize = {cache_ledger_query_batch_entries};\n\
             /// Compiled SQLite statements retained per connection.\n\
             pub const CACHE_SQLITE_PREPARED_STATEMENTS: usize = {cache_sqlite_prepared_statements};\n\
             /// Service storage diagnostic results retained.\n\
             pub const CACHE_SERVICE_STORAGE_DIAGNOSTICS_ENTRIES: usize = {cache_service_storage_diagnostics_entries};\n\
             /// Persistent-resume states retained by the service.\n\
             pub const CACHE_SERVICE_PERSISTENT_RESUME_ENTRIES: usize = {cache_service_persistent_resume_entries};\n\
             /// Reuse period for release update checks.\n\
             pub const CACHE_UPDATE_CHECK_TTL_SECONDS: u64 = {cache_update_check_ttl_seconds};\n\
             /// Reuse period for the last good image catalog.\n\
             pub const CACHE_IMAGE_CATALOG_REFRESH_SECONDS: u64 = {cache_image_catalog_refresh_seconds};\n\
             /// Retry delay after an image catalog failure.\n\
             pub const CACHE_IMAGE_CATALOG_RETRY_SECONDS: u64 = {cache_image_catalog_retry_seconds};\n\
             /// Catalog keys admitted to one cache-verification request.\n\
             pub const CACHE_IMAGE_VERIFICATION_BATCH_KEYS: usize = {cache_image_verification_batch_keys};\n{terminal_limits}{approval_limits}{update_limits}"
        ),
    )
    .expect("runtime limits: write generated constants");
    println!("cargo:rerun-if-changed={}", config_path.display());
}

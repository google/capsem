//! Compile-time hash of normalized protocol syntax.
//!
//! Rust parsing removes comments, token rendering normalizes formatting, and
//! only type/constant declarations enter the digest. Documentation, rustfmt,
//! and implementation-only edits therefore do not make compatible binaries
//! refuse each other. Serde attributes, field names, variants and framing
//! constants remain in the token stream and change the hash.

use quote::ToTokens;
use std::path::PathBuf;
use syn::visit_mut::VisitMut;

struct StripDocs;

impl VisitMut for StripDocs {
    fn visit_attributes_mut(&mut self, attributes: &mut Vec<syn::Attribute>) {
        attributes.retain(|attribute| !attribute.path().is_ident("doc"));
        for attribute in attributes {
            syn::visit_mut::visit_attribute_mut(self, attribute);
        }
    }
}

fn main() {
    let files = ["lib.rs", "ipc.rs", "handshake.rs", "router.rs", "exec_stream.rs"];
    write_hash("schema_hash.txt", &files);
    write_hash(
        "ledger_schema_hash.txt",
        &["ledger.rs", "ledger_commitment.rs", "ledger_control.rs"],
    );
    write_hash("proxy_schema_hash.txt", &["proxy_control.rs", "proxy_policy.rs"]);
    write_runtime_limits();
}

fn write_runtime_limits() {
    let config_path = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"))
        .join("../../config/data/runtime-limits.toml");
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
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    std::fs::write(
        format!("{out_dir}/runtime_limits.rs"),
        format!(
            "/// Supported concurrent network-client ceiling for one VM.\n\
             ///\n\
             /// TCP descriptor grants and each of the two DNS sessions derive\n\
             /// their bounds from this one config-owned value.\n\
             pub const NETWORK_CLIENT_CAPACITY: usize = {capacity};\n\
             /// Live and retiring traffic grants allowed during connection replacement.\n\
             pub const NETWORK_TRAFFIC_GRANT_CAPACITY: usize = {traffic_grant_capacity};\n\
             /// Minimum descriptor budget for a network-facing process.\n\
             pub const NETWORK_PROCESS_OPEN_FILE_LIMIT: u64 = {open_file_limit};\n\
             /// Per-producer security-event queue derived from supported clients.\n\
             pub const LEDGER_PRODUCER_QUEUE_CAPACITY: usize = {ledger_queue_capacity};\n\
             /// Consecutive producer events coalesced into one ledger exchange.\n\
             pub const LEDGER_ADMISSION_BATCH_CAPACITY: usize = {ledger_batch_capacity};\n\
             /// Maximum wait for an immediate follower before admitting a singleton.\n\
             pub const LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS: u64 = {ledger_coalesce_wait_microseconds};\n\
             /// Maximum time accepted rows stay only in the memory schema.\n\
             pub const LEDGER_DISK_FLUSH_INTERVAL_MILLISECONDS: u64 = {ledger_disk_flush_interval_milliseconds};\n\
             /// Pending events that force a disk flush before the timer.\n\
             pub const LEDGER_DISK_FLUSH_MAX_PENDING_EVENTS: usize = {ledger_disk_flush_max_pending_events};\n"
        ),
    )
    .expect("runtime limits: write generated constants");
    println!("cargo:rerun-if-changed={}", config_path.display());
}

fn write_hash(output: &str, files: &[&str]) {
    let mut hash = 0xcbf29ce484222325_u64;

    for f in files {
        let path = format!("src/{f}");
        let source = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("schema hash: read {path}: {error}"));
        let mut syntax = syn::parse_file(&source).unwrap_or_else(|error| panic!("schema hash: parse {path}: {error}"));
        StripDocs.visit_file_mut(&mut syntax);
        for item in syntax.items {
            if matches!(
                &item,
                syn::Item::Const(_)
                    | syn::Item::Enum(_)
                    | syn::Item::Static(_)
                    | syn::Item::Struct(_)
                    | syn::Item::Type(_)
                    | syn::Item::Union(_)
            ) {
                for byte in item.into_token_stream().to_string().bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }
            }
        }
        println!("cargo:rerun-if-changed={path}");
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    std::fs::write(format!("{out_dir}/{output}"), format!("{hash}u64")).expect("schema hash: write generated constant");
}

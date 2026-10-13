"""Keep cross-process network ceilings under one config authority."""

from __future__ import annotations

import re
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[2]
CONFIG = ROOT / "config/data/runtime-limits.toml"

RUNTIME_LIMIT_RATIONALE = """\
The proxy, trusted upstream broker, host DNS session, guest DNS forwarder and
security ledger must admit the same supported client burst. A pooled traffic
client can have an active, idle and retiring connection at once, and each
client can produce several audit rows, so downstream capacities must preserve
their config-owned multiples. Local numeric ceilings previously let one stage
accept work that the next stage shed, producing intermittent HTTP 502 and DNS
SERVFAIL responses under load.
config/data/runtime-limits.toml is the authority; capsem-proto generates the
shared constants and every consumer derives from them.
See skills/dev-mitm-proxy/SKILL.md under Cross-process network limits.
"""

NUMERIC_CONSTANT = re.compile(
    r"\b(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*(?:usize|u64)\s*=\s*(\d+)\s*;"
)
CONFIG_OWNED_NAMES = {
    "NETWORK_CLIENT_CAPACITY",
    "NETWORK_PROCESS_OPEN_FILE_LIMIT",
    "DNS_SESSION_MAX_IN_FLIGHT",
    "UPSTREAM_GRANT_CAPACITY",
    "TRAFFIC_GRANT_CAPACITY",
    "MAX_SELECTIONS",
    "MAX_ACTIVE_GRANTS",
    "LEDGER_PRODUCER_QUEUE_CAPACITY",
    "LEDGER_ADMISSION_BATCH_CAPACITY",
    "LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS",
}

REQUIRED_WIRING = {
    "crates/capsem-proto/build.rs": (
        "../../config/data/runtime-limits.toml",
        'get("max_concurrent_clients")',
        "pub const NETWORK_CLIENT_CAPACITY: usize = {capacity}",
        'get("traffic_grants_per_client")',
        ".checked_mul(traffic_grants_per_client)",
        "pub const NETWORK_TRAFFIC_GRANT_CAPACITY: usize = {traffic_grant_capacity}",
        'get("file_descriptors_per_client")',
        ".checked_mul(descriptors_per_client)",
        "pub const NETWORK_PROCESS_OPEN_FILE_LIMIT: u64 = {open_file_limit}",
        'get("producer_events_per_network_client")',
        ".checked_mul(producer_events_per_network_client)",
        'get("admission_batch_capacity")',
        "ledger_queue_capacity % ledger_batch_capacity",
        "pub const LEDGER_PRODUCER_QUEUE_CAPACITY: usize = {ledger_queue_capacity}",
        "pub const LEDGER_ADMISSION_BATCH_CAPACITY: usize = {ledger_batch_capacity}",
        'get("admission_coalesce_wait_microseconds")',
        "pub const LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS: u64 = {ledger_coalesce_wait_microseconds}",
    ),
    "crates/capsem-proto/src/lib.rs": (
        'include!(concat!(env!("OUT_DIR"), "/runtime_limits.rs"))',
        "DNS_SESSION_MAX_IN_FLIGHT: usize = NETWORK_CLIENT_CAPACITY",
    ),
    "crates/capsem-proto/src/upstream_grant.rs": (
        "UPSTREAM_GRANT_CAPACITY: usize = crate::NETWORK_TRAFFIC_GRANT_CAPACITY",
    ),
    "crates/capsem-proxy/src/main.rs": (
        "UPSTREAM_GRANT_CAPACITY",
        "NETWORK_PROCESS_OPEN_FILE_LIMIT",
        "LEDGER_PRODUCER_QUEUE_CAPACITY",
        "ensure_open_file_limit",
    ),
    "crates/capsem-service/src/main.rs": (
        "NETWORK_PROCESS_OPEN_FILE_LIMIT",
        "ensure_open_file_limit",
    ),
    "crates/capsem-service/src/upstream_broker.rs": ("UPSTREAM_GRANT_CAPACITY",),
    "crates/capsem-process/src/main.rs": (
        "NETWORK_PROCESS_OPEN_FILE_LIMIT",
        "LEDGER_PRODUCER_QUEUE_CAPACITY",
        "ensure_open_file_limit",
    ),
    "crates/capsem-logger/src/writer/remote.rs": (
        "LEDGER_ADMISSION_BATCH_CAPACITY",
        "LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS",
        "ReserveBatch",
        "AdmitBatch",
    ),
    "crates/capsem-logger/src/db.rs": ("LEDGER_PRODUCER_QUEUE_CAPACITY",),
    "crates/capsem-core/src/net/dns/session/mod.rs": ("DNS_SESSION_MAX_IN_FLIGHT",),
    "crates/capsem-agent/src/dns_proxy/session.rs": ("DNS_SESSION_MAX_IN_FLIGHT",),
    "crates/capsem-agent/src/net_proxy.rs": (
        "NETWORK_PROCESS_OPEN_FILE_LIMIT",
        "ensure_open_file_limit",
    ),
    "crates/capsem-bench/src/runtime_limits.rs": (
        "NETWORK_CLIENT_CAPACITY",
        "LEDGER_PRODUCER_QUEUE_CAPACITY",
        "LEDGER_ADMISSION_BATCH_CAPACITY",
        "LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS",
    ),
    "guest/artifacts/capsem_bench/dns_load.py": (
        '"runtime-limits"',
        '"network_client_capacity"',
        "capacity // 2",
    ),
}


def _numeric_limit_declarations(sources: dict[str, str]) -> list[str]:
    violations = []
    for path, source in sources.items():
        for name, value in NUMERIC_CONSTANT.findall(source):
            if name in CONFIG_OWNED_NAMES:
                violations.append(f"{path}: numeric {name} = {value}")
    return violations


def test_runtime_limit_config_is_valid_and_wired_to_every_consumer() -> None:
    config = tomllib.loads(CONFIG.read_text())
    capacity = config.get("network", {}).get("max_concurrent_clients")
    assert type(capacity) is int and 1 <= capacity <= 4096, (
        RUNTIME_LIMIT_RATIONALE
        + "\nnetwork.max_concurrent_clients must be an integer from 1 through 4096"
    )
    traffic_grants_per_client = config.get("network", {}).get(
        "traffic_grants_per_client"
    )
    assert (
        type(traffic_grants_per_client) is int and 3 <= traffic_grants_per_client <= 4
    ), (
        RUNTIME_LIMIT_RATIONALE
        + "\nnetwork.traffic_grants_per_client must cover active, idle and "
        "retiring pooled connections"
    )
    descriptors_per_client = config.get("network", {}).get(
        "file_descriptors_per_client"
    )
    assert type(descriptors_per_client) is int and 3 <= descriptors_per_client <= 64, (
        RUNTIME_LIMIT_RATIONALE
        + "\nnetwork.file_descriptors_per_client must cover TCP and VSOCK "
        "descriptors plus runtime overhead"
    )
    assert capacity * descriptors_per_client <= 1_048_576, RUNTIME_LIMIT_RATIONALE
    assert descriptors_per_client >= traffic_grants_per_client * 2, (
        RUNTIME_LIMIT_RATIONALE
        + "\nnetwork.file_descriptors_per_client must cover both ends of every "
        "overlapping traffic grant"
    )
    producer_events_per_network_client = config.get("ledger", {}).get(
        "producer_events_per_network_client"
    )
    assert (
        type(producer_events_per_network_client) is int
        and 1 <= producer_events_per_network_client <= 16
    ), RUNTIME_LIMIT_RATIONALE
    ledger_queue_capacity = capacity * producer_events_per_network_client
    assert ledger_queue_capacity <= 65_536, RUNTIME_LIMIT_RATIONALE
    admission_batch_capacity = config.get("ledger", {}).get("admission_batch_capacity")
    assert (
        type(admission_batch_capacity) is int
        and 1 <= admission_batch_capacity <= 1024
        and admission_batch_capacity <= ledger_queue_capacity
        and ledger_queue_capacity % admission_batch_capacity == 0
    ), RUNTIME_LIMIT_RATIONALE
    coalesce_wait_microseconds = config.get("ledger", {}).get(
        "admission_coalesce_wait_microseconds"
    )
    assert (
        type(coalesce_wait_microseconds) is int
        and 1 <= coalesce_wait_microseconds <= 10_000
    ), RUNTIME_LIMIT_RATIONALE

    missing = []
    for relative_path, required_fragments in REQUIRED_WIRING.items():
        source = (ROOT / relative_path).read_text()
        missing.extend(
            f"{relative_path}: missing {fragment!r}"
            for fragment in required_fragments
            if fragment not in source
        )
    assert not missing, RUNTIME_LIMIT_RATIONALE + "\n" + "\n".join(missing)


def test_runtime_limit_names_have_no_numeric_rust_authority() -> None:
    sources = {
        str(path.relative_to(ROOT)): path.read_text()
        for path in (ROOT / "crates").glob("*/src/**/*.rs")
    }
    violations = _numeric_limit_declarations(sources)
    assert not violations, RUNTIME_LIMIT_RATIONALE + "\n" + "\n".join(violations)


def test_numeric_local_ceiling_is_rejected_adversarially() -> None:
    violations = _numeric_limit_declarations(
        {
            "upstream_broker.rs": "const MAX_ACTIVE_GRANTS: usize = 256;",
            "dns/session.rs": "pub const DNS_SESSION_MAX_IN_FLIGHT: usize = 512;",
            "net_proxy.rs": "const NETWORK_PROCESS_OPEN_FILE_LIMIT: u64 = 4096;",
            "process.rs": "const LEDGER_PRODUCER_QUEUE_CAPACITY: usize = 1024;",
            "logger.rs": "const LEDGER_ADMISSION_BATCH_CAPACITY: usize = 64;",
            "remote.rs": "const LEDGER_ADMISSION_COALESCE_WAIT_MICROSECONDS: u64 = 100;",
        }
    )
    assert len(violations) == 6, RUNTIME_LIMIT_RATIONALE

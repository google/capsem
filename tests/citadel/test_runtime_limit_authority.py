"""Keep cross-process network ceilings under one config authority."""

from __future__ import annotations

import re
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[2]
CONFIG = ROOT / "config/data/runtime-limits.toml"

RUNTIME_LIMIT_RATIONALE = """\
The proxy, trusted upstream broker, host DNS session and guest DNS forwarder
must admit the same supported client burst. Local numeric ceilings previously
let one stage accept work that the next stage shed, producing intermittent HTTP
502 responses under load. config/data/runtime-limits.toml is the authority;
capsem-proto generates the shared constant and every consumer derives from it.
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
}

REQUIRED_WIRING = {
    "crates/capsem-proto/build.rs": (
        "../../config/data/runtime-limits.toml",
        'get("max_concurrent_clients")',
        "pub const NETWORK_CLIENT_CAPACITY: usize = {capacity}",
        'get("process_open_file_limit")',
        "pub const NETWORK_PROCESS_OPEN_FILE_LIMIT: u64 = {open_file_limit}",
    ),
    "crates/capsem-proto/src/lib.rs": (
        'include!(concat!(env!("OUT_DIR"), "/runtime_limits.rs"))',
        "DNS_SESSION_MAX_IN_FLIGHT: usize = NETWORK_CLIENT_CAPACITY",
    ),
    "crates/capsem-proto/src/upstream_grant.rs": (
        "UPSTREAM_GRANT_CAPACITY: usize = crate::NETWORK_CLIENT_CAPACITY",
    ),
    "crates/capsem-proxy/src/main.rs": ("UPSTREAM_GRANT_CAPACITY",),
    "crates/capsem-service/src/upstream_broker.rs": ("UPSTREAM_GRANT_CAPACITY",),
    "crates/capsem-core/src/net/dns/session/mod.rs": ("DNS_SESSION_MAX_IN_FLIGHT",),
    "crates/capsem-agent/src/dns_proxy/session.rs": ("DNS_SESSION_MAX_IN_FLIGHT",),
    "crates/capsem-agent/src/net_proxy.rs": (
        "NETWORK_PROCESS_OPEN_FILE_LIMIT",
        "ensure_open_file_limit",
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
    open_file_limit = config.get("network", {}).get("process_open_file_limit")
    assert type(open_file_limit) is int and 2 * capacity + 64 <= open_file_limit <= 1_048_576, (
        RUNTIME_LIMIT_RATIONALE
        + "\nnetwork.process_open_file_limit must cover TCP and VSOCK descriptors "
        "for every client plus runtime overhead"
    )

    missing = []
    for relative_path, required_fragments in REQUIRED_WIRING.items():
        source = (ROOT / relative_path).read_text()
        missing.extend(
            f"{relative_path}: missing {fragment!r}"
            for fragment in required_fragments
            if fragment not in source
        )
    assert not missing, RUNTIME_LIMIT_RATIONALE + "\n" + "\n".join(missing)


def test_network_limit_names_have_no_numeric_rust_authority() -> None:
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
        }
    )
    assert len(violations) == 3, RUNTIME_LIMIT_RATIONALE

"""Every production native policy owner replaces the non-nestable gate policy.

Missing an owner fails only after the expensive runtime rebuild and VM boot.
Classify every actual worker role here; external MCP programs inherit their
worker's restrictions and must never receive a trusted executable handoff.
"""

import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OWNERS = {
    "Gateway": "capsem-gateway",
    "Ledger": "capsem-ledger",
    "Proxy": "capsem-proxy",
    "VmOwner": "capsem-process",
    "McpAggregator": "capsem-mcp-aggregator",
    "McpBuiltin": "capsem-mcp-builtin",
    "McpExternal": None,
}


def test_every_native_role_has_its_exact_gate_handoff_and_external_tools_inherit():
    source = (ROOT / "crates/capsem-foundation/src/unix/worker_sandbox.rs").read_text()
    role = source.split("pub enum Role {", 1)[1].split("\n}", 1)[0]
    variants = set(re.findall(r"^\s+([A-Za-z]+),$", role, re.MULTILINE))
    assert variants == OWNERS.keys(), "classify every real native policy owner before qualification"
    config = tomllib.loads((ROOT / "config/gate.toml").read_text())
    actual = set(config["sandbox"]["self_confined_executables"])
    expected = {name for name in OWNERS.values() if name is not None}
    expected |= {"capsem-router", "capsem-router-confinement-test", "capsem-worker-confinement-test", "capsem-gateway-confinement-test", "capsem-mcp-confinement-test"}
    assert actual == expected, (
        "Seatbelt cannot nest: every native role must install its own strict policy before readiness; "
        f"missing={expected - actual}, unexpected={actual - expected}"
    )

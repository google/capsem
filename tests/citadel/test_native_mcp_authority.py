"""Keep actual native MCP startup and retirement inside the worker boundary."""

from __future__ import annotations

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
PATHS = {
    "aggregator_main": "crates/capsem-mcp-aggregator/src/main.rs",
    "aggregator": "crates/capsem-mcp-aggregator/src/runtime.rs",
    "manager": "crates/capsem-mcp-aggregator/src/server_manager.rs",
    "builtin": "crates/capsem-mcp-builtin/src/main.rs",
    "bootstrap": "crates/capsem-mcp-transport/src/native_worker.rs",
    "owner": "crates/capsem-process/src/aggregator.rs",
    "supervisor": "crates/capsem-service/src/upstream_broker/mcp_native.rs",
    "retirement": "crates/capsem-service/src/instance_reaper/retirement.rs",
    "provision": "crates/capsem-service/src/vm_lifecycle/provision.rs",
    "resume": "crates/capsem-service/src/vm_lifecycle/resume_process.rs",
}
NATIVE_AUTHORITY_RATIONALE = """\
Separate PIDs and parent watching never confined the MCP children (#353).
Native workers must clear inherited authority, install and attest their OS
sandbox before Ready, and use only connected async capabilities. The service
must retain child handles and await their cleanup before generation retirement;
an aborted task or VM-owner exit alone is not positive child-reaping evidence.
See skills/dev-mcp/SKILL.md under MCP subprocess architecture. These fast source
guards complement actual native tool/denial tests and final platform proof.
"""


def sources() -> dict[str, str]:
    return {name: (ROOT / path).read_text() for name, path in PATHS.items()}


def code(source: str) -> str:
    return re.sub(r"\s+", "", re.sub(r"(?m)^\s*//.*$", "", source))


def body(source: str, name: str) -> str:
    match = re.search(rf"\bfn\s+{re.escape(name)}\b[^{{]*\{{", source)
    if match is None:
        return ""
    start = match.end()
    depth = 1
    for index in range(start, len(source)):
        depth += (source[index] == "{") - (source[index] == "}")
        if depth == 0:
            return code(source[start:index])
    return ""


def ordered(source: str, *tokens: str) -> bool:
    end = 0
    for token in tokens:
        found = source.find(token, end)
        if found < 0:
            return False
        end = found + len(token)
    return True


def violations(raw: dict[str, str]) -> list[str]:
    problems = []
    for name in ("aggregator_main", "builtin"):
        main = body(raw[name], "main")
        if not ordered(main, "clear_inherited_environment()", "close_inherited_descriptors()", "new_current_thread()", ".block_on("):
            problems.append(f"{name}: startup retained inherited authority or created threads before confinement")
        if "#[tokio::main" in code(raw[name]):
            problems.append(f"{name}: runtime starts before entrypoint authority cleanup")
    for name, role in (("aggregator", "Aggregator"), ("builtin", "Builtin")):
        run = body(raw[name], "run")
        if not ordered(run, f"PreparedMcpWorker::from_stdin(McpHttpRole::{role})", ".confine_and_ready()", "watch_parent_or_exit("):
            problems.append(f"{name}: traffic/parent watcher can start without enforced confinement")
    bootstrap = body(raw["bootstrap"], "confine_and_ready")
    if not ordered(bootstrap, "worker_sandbox::confine(", "worker_sandbox::attest_descriptor_only()", "McpWorkerRecord::Ready(", "self.control.write_all("):
        problems.append("bootstrap: readiness precedes confinement or actual denial attestation")
    for call in ("confine", "attest_descriptor_only"):
        if not re.search(rf"worker_sandbox::{call}\([^;]+\?;", bootstrap):
            problems.append(f"bootstrap: {call} failure is not propagated before readiness")
    aggregator = code(raw["aggregator"])
    for forbidden in ("tokio::io::stdin()", "tokio::io::stdout()", "McpServerManager::new("):
        if forbidden in aggregator:
            problems.append(f"aggregator: ambient/blocking runtime path {forbidden}")
    if "let(mutstdin,mutstdout)=worker.control.into_split()" not in aggregator:
        problems.append("aggregator: RPC must use both async halves of its connected descriptor")
    if "manager.capabilities=self.capabilities.clone()" not in body(raw["manager"], "replacement"):
        problems.append("manager: refresh drops inherited capabilities")
    owner = code(raw["owner"])
    if "upstream.open_mcp_aggregator()" not in owner or "::Command::" in owner:
        problems.append("owner: native MCP spawning/control escaped its data-only grant")
    supervisor = body(raw["supervisor"], "run")
    if not ordered(supervisor, "self.scope.revoke()", "self.child.start_kill()", "self.child.wait()", "(&mutself.spawn).await"):
        problems.append("service: native child handles are not reaped before supervisor completion")
    retirement = body(raw["retirement"], "wait_child_before_reap")
    if not ordered(retirement, "child.wait().await", "self.brokers.drain(..)", "completion.cleanup_confirmed", "status.map("):
        problems.append("retirement: native exit can complete before confirmed broker cleanup")
    for name in ("provision", "resume"):
        if not ordered(code(raw[name]), "retirement.with_brokers(", "instance_reaper::spawn_exit_reaper("):
            problems.append(f"{name}: generation reaper does not own broker completion")
    return problems


def test_native_mcp_entrypoints_and_retirement_keep_enforced_authority() -> None:
    assert not (problems := violations(sources())), f"{problems}\n{NATIVE_AUTHORITY_RATIONALE}"


@pytest.mark.parametrize(
    ("name", "before", "after"),
    [
        ("aggregator_main", "clear_inherited_environment", "retain_environment"),
        ("builtin", "new_current_thread", "new_multi_thread"),
        ("aggregator", ".confine_and_ready()", ".accept_without_confinement()"),
        ("bootstrap", "worker_sandbox::confine(", "unchecked_profile("),
        ("bootstrap", "worker_sandbox::attest_descriptor_only()", "skip_attestation()"),
        ("bootstrap", '.context("install native MCP sandbox")?', ".ok()"),
        ("aggregator", "worker.control.into_split()", "ambient_stdio()"),
        ("manager", "self.capabilities.clone()", "None"),
        ("owner", "upstream.open_mcp_aggregator()", "tokio::process::Command::new(\"worker\")"),
        ("supervisor", "(&mut self.spawn).await", "self.spawn.abort()"),
        ("retirement", "self.brokers.drain(..)", "std::iter::empty()"),
        ("resume", "retirement.with_brokers(", "detach_brokers("),
    ],
)
def test_native_mcp_guard_rejects_authority_regressions(name: str, before: str, after: str) -> None:
    raw = sources()
    assert before in raw[name], f"adversarial fixture lost its source anchor: {before}"
    raw[name] = raw[name].replace(before, after, 1)
    assert violations(raw), NATIVE_AUTHORITY_RATIONALE


def test_native_mcp_guard_rejects_attestation_before_confinement() -> None:
    raw = sources()
    lines = raw["bootstrap"].splitlines()
    confine = next(index for index, line in enumerate(lines) if "worker_sandbox::confine(" in line)
    attest = next(index for index, line in enumerate(lines) if "worker_sandbox::attest_descriptor_only()" in line)
    lines[confine], lines[attest] = lines[attest], lines[confine]
    raw["bootstrap"] = "\n".join(lines)
    assert violations(raw), NATIVE_AUTHORITY_RATIONALE

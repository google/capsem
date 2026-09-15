"""Citadel guard: only the service and the VM owner speak the owner protocol.

`capsem create/run --image` and `capsem doctor` once opened the per-VM owner
socket straight from the CLI: publishing ports and attaching with
`ServiceToProcess` messages the service never saw, while the gateway read the
socket path back out of `ProvisionResponse.uds_path` and relayed it to remote
clients. Remote SDKs, the browser and MCP could not take that path, and the
service's lifecycle, policy and audit were bypassed on it (google/capsem#207).
Every client now goes through service routes; this keeps it that way.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]

RATIONALE = """\
Only capsem-service and capsem-process talk to a VM owner.

Clients (CLI, gateway, TUI, tray, desktop app, MCP) use service HTTP routes:
/vms/{id}/container, /vms/{id}/exposures and the capsem.stream.v1 WebSocket
at /vms/{id}/stream. The owner IPC protocol (ServiceToProcess and
ProcessToService), the per-VM socket paths and the owner handshake stay
inside the service, the owner and the library crates they are built from, so
lifecycle, policy and audit cannot be skipped by a client that dials the
socket itself.
"""

#: Crates allowed to name the owner protocol: the two ends, and the libraries
#: that define and implement it for them.
OWNERS = frozenset({"capsem-service", "capsem-process", "capsem-core", "capsem-proto", "capsem-foundation"})

OWNER_PROTOCOL = re.compile(
    r"\b(ServiceToProcess|ProcessToService|instance_socket_path|private_handoff_socket_path"
    r"|negotiate_initiator(?:_off_worker)?|STREAM_PEER_ID)\b"
)


def _code(text: str) -> list[tuple[int, str]]:
    """Lines with `//` comments removed; string literals are kept, since a
    path spelled in a string is still a dial."""
    lines = []
    for number, line in enumerate(text.splitlines(), start=1):
        code = line.split("//", 1)[0].strip()
        if code:
            lines.append((number, code))
    return lines


def owner_protocol_references(root: Path = PROJECT_ROOT) -> list[str]:
    found = []
    for path in sorted((root / "crates").rglob("*.rs")):
        crate = path.relative_to(root / "crates").parts[0]
        if crate in OWNERS:
            continue
        for number, code in _code(path.read_text(encoding="utf-8")):
            if OWNER_PROTOCOL.search(code):
                found.append(f"{path.relative_to(root)}:{number}: {code}")
    return found


def owner_socket_suffixes(root: Path = PROJECT_ROOT) -> list[str]:
    """A client that rebuilds `instances/<id>.sock` by hand dials the owner
    without naming any helper."""
    pattern = re.compile(r"instances/|-handoff\.sock|\{id\}\.sock")
    found = []
    for path in sorted((root / "crates").rglob("*.rs")):
        crate = path.relative_to(root / "crates").parts[0]
        if crate in OWNERS:
            continue
        for number, code in _code(path.read_text(encoding="utf-8")):
            if pattern.search(code):
                found.append(f"{path.relative_to(root)}:{number}: {code}")
    return found


def test_clients_never_name_the_owner_protocol() -> None:
    found = owner_protocol_references()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_clients_never_rebuild_an_owner_socket_path() -> None:
    found = owner_socket_suffixes()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def _crate(root: Path, name: str, source: str) -> None:
    src = root / "crates" / name / "src"
    src.mkdir(parents=True, exist_ok=True)
    (src / "main.rs").write_text(source, encoding="utf-8")


def test_evasions_are_reported(tmp_path: Path) -> None:
    _crate(
        tmp_path,
        "capsem",
        "// ServiceToProcess in prose is fine\n"
        "use capsem_proto::ipc::{ProcessToService as Reply, ServiceToProcess};\n"
        "let path = capsem_foundation::uds::instance_socket_path(&run, &id)?;\n"
        "negotiate_initiator_off_worker(socket, \"capsem-cli\", None).await?;\n",
    )
    _crate(tmp_path, "capsem-gateway", "let p = run_dir.join(format!(\"instances/{id}.sock\"));\n")
    _crate(tmp_path, "capsem-tui", 'let p = format!("{dir}/{id}-handoff.sock");\n')
    _crate(tmp_path, "capsem-service", "send(ServiceToProcess::Ping);\n")
    assert owner_protocol_references(tmp_path) == [
        "crates/capsem/src/main.rs:2: use capsem_proto::ipc::{ProcessToService as Reply, ServiceToProcess};",
        "crates/capsem/src/main.rs:3: let path = capsem_foundation::uds::instance_socket_path(&run, &id)?;",
        'crates/capsem/src/main.rs:4: negotiate_initiator_off_worker(socket, "capsem-cli", None).await?;',
    ]
    assert owner_socket_suffixes(tmp_path) == [
        'crates/capsem-gateway/src/main.rs:1: let p = run_dir.join(format!("instances/{id}.sock"));',
        'crates/capsem-tui/src/main.rs:1: let p = format!("{dir}/{id}-handoff.sock");',
    ]

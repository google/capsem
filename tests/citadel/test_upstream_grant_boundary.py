"""Citadel guard for generation-bound upstream descriptor grants.

The VM owner may name a host, protocol and configured DNS index.  Only the
trusted service resolves or connects those names.  The service hands the owner
a connected descriptor and never relays workload bytes, keeping policy and
telemetry on the trusted side without putting traffic on the control channel.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]

RATIONALE = """\
Confined VM owners obtain upstream access only through generation-bound grants.

capsem-process may pass its generation-bound grant channel to capsem-proxy, but
neither worker may resolve hosts, bind an outbound socket, or dial an address.
The trusted service selects the peer from its active policy, opens the socket,
and passes the descriptor. MITM request, retry and WebSocket paths consume it.
Payload bytes stay on the connected descriptor and never cross the coordinator
control channel.  This prevents arbitrary egress, stale-generation reuse and a
high-bandwidth workload relay through the trusted service.
"""

DIRECT_UPSTREAM = re.compile(
    r"\b(?:TcpStream|UdpSocket|TcpSocket|lookup_host|ToSocketAddrs)\b"
    r"|\bUpstreamResolver::system\b|\bUpstreamTarget::resolve\b"
)

# These are descriptor adoption only.  The exact source lines make a new
# constructor, alias or socket operation a reviewed change to this boundary.
ALLOWED_PROCESS_SOCKET_LINES = {
    "crates/capsem-process/src/cables.rs": {
        "let source = capsem_core::container::publish::Source(std::net::TcpStream::from("
    },
    "crates/capsem-process/src/owner_sandbox.rs": {
        "std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 9)).map(|_| ()),",
    },
    "crates/capsem-proxy/src/main.rs": {
        "std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 9)),",
    },
}


def _production_rust(root: Path) -> list[Path]:
    return [
        path
        for crate in ("capsem-process", "capsem-proxy")
        for path in sorted((root / "crates" / crate / "src").rglob("*.rs"))
        if "tests" not in path.relative_to(root / "crates" / crate / "src").parts
        and path.name != "tests.rs"
    ]


def _code_lines(path: Path) -> list[tuple[int, str]]:
    lines = []
    for number, source in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        code = source.split("//", 1)[0].strip()
        if code:
            lines.append((number, code))
    return lines


def process_direct_upstream_references(root: Path = PROJECT_ROOT) -> list[str]:
    found = []
    for path in _production_rust(root):
        relative = path.relative_to(root).as_posix()
        allowed = ALLOWED_PROCESS_SOCKET_LINES.get(relative, set())
        for number, code in _code_lines(path):
            if DIRECT_UPSTREAM.search(code) and code not in allowed:
                found.append(f"{relative}:{number}: {code}")
    return found


def test_workers_can_only_adopt_upstream_descriptors() -> None:
    found = process_direct_upstream_references()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_proxy_installs_grants_for_dns_and_mitm() -> None:
    source = (PROJECT_ROOT / "crates/capsem-proxy/src/main.rs").read_text(encoding="utf-8")
    assert "DnsResolver::with_grants(" in source, RATIONALE
    assert "upstream_grants: Some(" in source, RATIONALE
    assert "Arc<dyn capsem_core::net::dns::DnsUpstreamGrants>" in source, RATIONALE
    assert "Arc<dyn capsem_core::net::mitm_proxy::TcpUpstreamGrants>" in source, RATIONALE


def test_every_mitm_dial_path_consumes_a_grant() -> None:
    mitm = PROJECT_ROOT / "crates/capsem-core/src/net/mitm_proxy"
    paths = [mitm / "mod.rs", mitm / "upgrade.rs"]
    source = "\n".join(path.read_text(encoding="utf-8") for path in paths)
    assert source.count(
        ".connect_with_grants(config.upstream_grants.as_deref(), policy_snapshot.digest())"
    ) == 3, RATIONALE
    assert not re.search(r"\btarget\s*\.\s*connect\s*\(", source), RATIONALE


def test_service_broker_opens_sockets_without_relaying_bodies() -> None:
    source = (PROJECT_ROOT / "crates/capsem-service/src/upstream_broker.rs").read_text(encoding="utf-8")
    for operation in (
        "UpstreamTarget::resolve(",
        "selection.target.connect()",
        "tokio::net::UdpSocket::bind(",
        "socket.connect(upstream)",
    ):
        assert operation in source, RATIONALE
    forbidden_relay = re.compile(
        r"\bcopy_bidirectional\b|\bAsyncReadExt\b|\bAsyncWriteExt\b"
        r"|\.read_exact\s*\(|\.write_all\s*\("
    )
    assert not forbidden_relay.search(source), RATIONALE


def test_process_socket_evasions_are_reported(tmp_path: Path) -> None:
    source = tmp_path / "crates/capsem-process/src"
    source.mkdir(parents=True)
    (source / "main.rs").write_text(
        "use tokio::net::TcpStream as Dialer;\n"
        'let tcp = Dialer::connect("203.0.113.7:443").await?;\n'
        'let dns = std::net::UdpSocket::bind("0.0.0.0:0")?;\n'
        "let target = UpstreamTarget::resolve(&UpstreamResolver::system(), policy, host, port).await;\n",
        encoding="utf-8",
    )
    assert process_direct_upstream_references(tmp_path) == [
        "crates/capsem-process/src/main.rs:1: use tokio::net::TcpStream as Dialer;",
        'crates/capsem-process/src/main.rs:3: let dns = std::net::UdpSocket::bind("0.0.0.0:0")?;',
        "crates/capsem-process/src/main.rs:4: let target = UpstreamTarget::resolve(&UpstreamResolver::system(), policy, host, port).await;",
    ]

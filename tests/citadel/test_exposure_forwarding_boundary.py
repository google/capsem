"""Citadel guard: exposure bytes cross only the confined router.

A prototype of google/capsem#207 reached exposed guest ports through the
service and gateway: a reverse proxy that read and wrote workload bytes in
processes holding credentials, the session ledger and every VM's lifecycle.
The shipped design keeps the privilege split of the network router: the
service decides and sends bounded IPC, the VM owner binds the loopback
listener and connects the authorized guest target, and only the
environment-cleared, sandbox-confirmed `capsem-router` copies bytes between
the two connected descriptors it is handed.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[2]

RATIONALE = """\
Exposure bytes are carried only by the confined capsem-router.

capsem-service authorizes exposures and talks to the VM owner over typed IPC.
capsem-gateway may accept a loopback browser socket and read bounded control
material needed to authenticate it, but hands that same descriptor to the VM
owner without opening the guest destination or carrying workload bytes. The VM
owner binds loopback TCP exposures, connects the authorized guest target, and
grants connected descriptor pairs to a router spawned with a cleared
environment, "/" as its directory, no stdout, and a confirmed sandbox. Do not
add a service or gateway reverse proxy, and do not give the router privileged
owners, listeners, or destination choice.
"""

#: Service modules that own exposure, stream and container control.
SERVICE_CONTROL = (
    "crates/capsem-service/src/router_runtime/exposures.rs",
    "crates/capsem-service/src/router_runtime/streams.rs",
    "crates/capsem-service/src/container_setup.rs",
)
TCP = re.compile(r"\bTcp(?:Stream|Listener|Socket)\b")
BYTE_COPY = re.compile(
    r"\bcopy_bidirectional(?:_with_sizes)?\b|\bio::copy_buf\b|\btokio::io::copy\b"
)

#: The gateway's only byte-copying site: the stream upgrade tunnel.
GATEWAY_TUNNEL = "crates/capsem-gateway/src/stream.rs"
#: The gateway owns its authenticated control listener and the loopback-only
#: browser admission listener. The latter hands admitted descriptors onward.
GATEWAY_LISTENER = (
    "crates/capsem-gateway/src/listener.rs",
    "crates/capsem-gateway/src/main.rs",
    "crates/capsem-gateway/src/preview.rs",
)


def _code(path: Path) -> list[tuple[int, str]]:
    lines = []
    for number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), start=1
    ):
        code = line.split("//", 1)[0].strip()
        if code:
            lines.append((number, code))
    return lines


def forwarding_in_service_control(root: Path = PROJECT_ROOT) -> list[str]:
    found = []
    for relative in SERVICE_CONTROL:
        path = root / relative
        for number, code in _code(path):
            if TCP.search(code) or BYTE_COPY.search(code):
                found.append(f"{relative}:{number}: {code}")
    return found


def forwarding_in_gateway(root: Path = PROJECT_ROOT) -> list[str]:
    found = []
    for path in sorted((root / "crates" / "capsem-gateway" / "src").rglob("*.rs")):
        relative = str(path.relative_to(root))
        if "/tests" in relative or relative.endswith("/tests.rs"):
            continue
        for number, code in _code(path):
            outbound_tcp = TCP.search(code) and relative not in GATEWAY_LISTENER
            connect = re.search(r"\bTcpStream::connect\b", code)
            copy = BYTE_COPY.search(code) and relative != GATEWAY_TUNNEL
            if outbound_tcp or connect or copy:
                found.append(f"{relative}:{number}: {code}")
    return found


def test_service_control_forwards_no_bytes() -> None:
    found = forwarding_in_service_control()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_gateway_forwards_only_the_stream_tunnel() -> None:
    found = forwarding_in_gateway()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_the_gateway_tunnel_is_the_stream_upgrade_to_the_service_socket() -> None:
    tunnel = (PROJECT_ROOT / GATEWAY_TUNNEL).read_text(encoding="utf-8")
    assert "UnixStream::connect(state.uds_path" in tunnel, RATIONALE
    assert "SWITCHING_PROTOCOLS" in tunnel and "hyper::upgrade::on" in tunnel, RATIONALE


def test_vm_owner_binds_loopback_and_grants_connected_pairs() -> None:
    publisher = (
        PROJECT_ROOT / "crates/capsem-core/src/container/publish.rs"
    ).read_text(encoding="utf-8")
    broker = (
        PROJECT_ROOT / "crates/capsem-core/src/container/publish/broker.rs"
    ).read_text(encoding="utf-8")
    assert "TcpListener::bind((Ipv4Addr::LOCALHOST, host_port))" in publisher, RATIONALE
    grant = " ".join(broker.split())
    assert ".grant( flow.source.as_fd(), destination.as_fd()," in grant, RATIONALE


def test_gateway_preview_hands_the_admitted_descriptor_to_the_owner() -> None:
    preview = (PROJECT_ROOT / "crates/capsem-gateway/src/preview.rs").read_text(
        encoding="utf-8"
    )
    assert "source.as_raw_fd()" in preview and "Sender::new" in preview, RATIONALE
    assert "SEAT_PREVIEW" in preview and "handoff_token" in preview, RATIONALE


def test_router_spawn_is_environment_cleared_and_confirmed_confined() -> None:
    spawn = (PROJECT_ROOT / "crates/capsem-core/src/net/router_process.rs").read_text(
        encoding="utf-8"
    )
    for required in (
        ".env_clear()",
        '.current_dir("/")',
        ".stdout(Stdio::null())",
        "Event::ConfinementFailed",
    ):
        assert required in spawn, RATIONALE + f"\nmissing {required}"
    companion = (
        PROJECT_ROOT / "crates/capsem-core/src/container/publish/companion.rs"
    ).read_text(encoding="utf-8")
    assert '"--expose-limit"' in companion, RATIONALE
    for secret in ("admin_token", "password", "ca_pem", "registry", "entitlement"):
        assert secret not in companion.lower(), (
            RATIONALE + f"\nrouter spawn names {secret}"
        )


def _runtime_dependencies(table: dict[str, Any]) -> set[str]:
    dependencies: set[str] = set()
    for key, value in table.items():
        if key in {"dependencies", "build-dependencies"} and isinstance(value, dict):
            dependencies.update(
                str(declaration.get("package", alias))
                if isinstance(declaration, dict)
                else alias
                for alias, declaration in value.items()
            )
        elif key != "dev-dependencies" and isinstance(value, dict):
            dependencies.update(_runtime_dependencies(value))
    return dependencies


def test_router_cannot_acquire_privileged_owners_or_listeners() -> None:
    manifest = tomllib.loads(
        (PROJECT_ROOT / "crates/capsem-router/Cargo.toml").read_text(encoding="utf-8")
    )
    privileged = {
        "capsem-config",
        "capsem-core",
        "capsem-credentials",
        "capsem-process",
        "capsem-service",
        "capsem-api",
    }
    assert not _runtime_dependencies(manifest) & privileged, RATIONALE
    main = (PROJECT_ROOT / "crates/capsem-router/src/main.rs").read_text(
        encoding="utf-8"
    )
    assert (
        "router_sandbox::close_inherited_descriptors" in main
        and "router_sandbox::confine" in main
    ), RATIONALE
    assert not TCP.search(main), RATIONALE


def test_evasions_are_reported(tmp_path: Path) -> None:
    for relative, source in {
        SERVICE_CONTROL[
            0
        ]: "// TcpStream in prose is fine\nlet s = tokio::net::TcpStream::connect(addr).await?;\n",
        SERVICE_CONTROL[1]: "tokio::io::copy_bidirectional(&mut a, &mut b).await?;\n",
        SERVICE_CONTROL[2]: "use std::net::TcpListener as L;\n",
        "crates/capsem-gateway/src/proxy.rs": 'let s = TcpStream::connect(("127.0.0.1", port)).await?;\n',
        "crates/capsem-gateway/src/main.rs": "tokio::io::copy_bidirectional_with_sizes(&mut a, &mut b, 1, 1).await?;\n",
        "crates/capsem-gateway/src/preview.rs": (
            "let s = TcpStream::connect(addr).await?;\n"
            "tokio::io::copy_bidirectional(&mut a, &mut b).await?;\n"
        ),
        GATEWAY_TUNNEL: "tokio::io::copy_bidirectional(&mut client, &mut service).await?;\n",
    }.items():
        path = tmp_path / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8")
    assert forwarding_in_service_control(tmp_path) == [
        f"{SERVICE_CONTROL[0]}:2: let s = tokio::net::TcpStream::connect(addr).await?;",
        f"{SERVICE_CONTROL[1]}:1: tokio::io::copy_bidirectional(&mut a, &mut b).await?;",
        f"{SERVICE_CONTROL[2]}:1: use std::net::TcpListener as L;",
    ]
    assert forwarding_in_gateway(tmp_path) == [
        "crates/capsem-gateway/src/main.rs:1: tokio::io::copy_bidirectional_with_sizes(&mut a, &mut b, 1, 1).await?;",
        "crates/capsem-gateway/src/preview.rs:1: let s = TcpStream::connect(addr).await?;",
        "crates/capsem-gateway/src/preview.rs:2: tokio::io::copy_bidirectional(&mut a, &mut b).await?;",
        'crates/capsem-gateway/src/proxy.rs:1: let s = TcpStream::connect(("127.0.0.1", port)).await?;',
    ]

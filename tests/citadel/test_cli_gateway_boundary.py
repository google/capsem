"""#347: ordinary CLI clients share authenticated gateway authority."""

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "crates/capsem"
CLI_GATEWAY_RATIONALE = (
    "#347/#353 require ordinary CLI operations to use SDK gateway authentication. "
    "UDS is reserved for explicit local administration/startup; authentication "
    "failure must never select a direct-service VM transport."
)
ORDINARY = (
    "route_ids.rs", "create_command.rs", "container_image.rs", "container_run.rs",
    "image_commands.rs", "network_commands.rs", "proxy_command.rs", "client/networks.rs",
)


def gateway_violations(source: str) -> list[str]:
    forbidden = (
        r"\bUnixStream\b",
        r"\bhyper\s*::",
        r"\bbootstrap\s*\.\s*(?:request(?:_bytes)?|get|post|delete)\s*(?:::|\()",
    )
    return [pattern for pattern in forbidden if re.search(pattern, source)]


def test_ordinary_cli_handlers_cannot_take_a_direct_service_client() -> None:
    for relative in ORDINARY:
        text = (CLI / "src" / relative).read_text()
        assert "UdsClient" not in text and "ServiceAdminClient" not in text, f"{relative}: {CLI_GATEWAY_RATIONALE}"
    main = (CLI / "src/main.rs").read_text()
    assert "let client = GatewayClient::with_direct_service_lifetime" in main, CLI_GATEWAY_RATIONALE


def test_cli_reuses_the_sdk_and_has_no_gateway_failure_fallback() -> None:
    assert "capsem-sdk" in (CLI / "Cargo.toml").read_text(), CLI_GATEWAY_RATIONALE
    gateway = (CLI / "src/client/gateway.rs").read_text()
    assert "capsem_sdk::transport" in gateway, CLI_GATEWAY_RATIONALE
    assert not gateway_violations(gateway), CLI_GATEWAY_RATIONALE
    stream = (CLI / "src/client/stream.rs").read_text()
    assert "capsem_sdk::transport" in stream, CLI_GATEWAY_RATIONALE
    assert "tokio_tungstenite" not in stream, CLI_GATEWAY_RATIONALE


@pytest.mark.parametrize("fallback", [
    'self.bootstrap.get("/vms/list")',
    'self.bootstrap . post::<_, Value>("/vms/create", body)',
    'self.bootstrap\n . request_bytes("POST", path, body)',
    'self.bootstrap.delete::<Value>(path)',
    'UnixStream::connect(path)',
    'hyper :: client::conn::http1::handshake(stream)',
])
def test_guard_rejects_direct_service_fallbacks(fallback: str) -> None:
    production = (CLI / "src/client/gateway.rs").read_text()
    assert gateway_violations(production + "\n" + fallback), CLI_GATEWAY_RATIONALE

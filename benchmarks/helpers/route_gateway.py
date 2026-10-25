"""Gateway-only static/health contracts; no preview bootstrap or rendering."""
from __future__ import annotations

import json
import re
import tomllib
from functools import lru_cache
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GATEWAY_RESPONSES = {
    "/": "health", "/health": "health", "/openapi.json": "openapi",
    "/vms/{id}/surface/": "html", "/vms/{id}/surface/launch.js": "javascript",
}


@lru_cache(maxsize=2)
def expected_artifact(response: str) -> bytes:
    source = ROOT / "crates/capsem-gateway/src/surface/static"
    if response == "javascript":
        return (source / "launch.js").read_bytes()
    assert response == "html"
    return (source / "launcher.html").read_text().replace("{{auto}}", "true").replace("{{parent}}", "").replace("{{profile}}", "auto").encode()


@lru_cache(maxsize=1)
def expected_schema() -> dict:
    return json.loads((ROOT / "sdk/specification/openapi.json").read_bytes())


@lru_cache(maxsize=1)
def expected_version() -> str:
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def validate_gateway(response: str, body: bytes, headers, service_socket: str | None) -> None:
    """Validate after receipt clocks, against selected first-party source."""
    assert "content-encoding" not in headers
    if response in {"health", "openapi"}:
        assert headers["content-type"] == "application/json"
        payload = json.loads(body)
        if response == "health":
            assert service_socket, "health needs the owning service socket"
            assert payload["ok"] is True
            assert payload == {"ok": True, "version": expected_version(), "service_socket": service_socket}
        else:
            assert payload == expected_schema(), "selected OpenAPI operations/schemas/version differ"
        return
    assert response in {"html", "javascript"}
    assert body == expected_artifact(response), "first-party static artifact differs"
    for name, value in {
        "content-type": "text/html; charset=utf-8" if response == "html" else "text/javascript; charset=utf-8",
        "x-content-type-options": "nosniff", "x-frame-options": "DENY",
        "referrer-policy": "no-referrer", "cache-control": "no-store",
        "cross-origin-opener-policy": "same-origin",
    }.items():
        assert headers[name] == value, f"invalid surface header: {name}"
    match = re.fullmatch(
        r"default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; "
        r"form-action http://\*\.localhost:([0-9]+); frame-ancestors 'none'; base-uri 'none'",
        headers["content-security-policy"],
    )
    assert match and 0 < int(match[1]) <= 65535, "unsafe surface policy"

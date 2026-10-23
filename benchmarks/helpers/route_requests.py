"""Typed route request inputs and post-clock validation, using one HTTP owner."""
from __future__ import annotations

import json
import tomllib
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

from helpers.http_measurement import MeasuredResponse
from helpers.http_transport import Transport
from helpers.stopped_workspace import BODY_URL, assert_event_bodies
from psutil import Process

from .route_binary import validate_archive, validate_file, validate_full_event
from .route_gateway import GATEWAY_RESPONSES, validate_gateway
from .route_measurements import validate_receipt

ROOT = Path(__file__).resolve().parents[2]
MANAGED_TOKEN = "af" * 32

def route_load_config() -> dict[str, int | float]:
    config = tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
    return dict(config["benchmark"]["route_load"])


def request_input(path: str) -> tuple[str, bytes | None]:
    """Return the method and body for one repeatable benchmark request."""
    if path.endswith("/claim"):
        return "POST", json.dumps({"ownership_token": MANAGED_TOKEN}).encode()
    if path.endswith("/close"):
        payload = {"ownership_token": MANAGED_TOKEN, "outcome": "cancelled"}
        return "POST", json.dumps(payload).encode()
    return "GET", None


@dataclass(frozen=True)
class Target:
    label: str
    transport_factory: Callable[[], Transport]
    headers: dict[str, str]
    cpu_processes: dict[str, Process]
    limits: dict[str, int | float] = field(default_factory=route_load_config)
    service_socket: str | None = None


def request(transport: Transport, target: Target, path: str, response: str = "json") -> MeasuredResponse:
    """Require one complete first-attempt JSON/text receipt."""
    if response in GATEWAY_RESPONSES.values() and target.label != "gateway":
        raise ValueError("gateway artifact cannot be measured over service UDS")
    method, request_body = request_input(path)
    accept = {"binary": "application/octet-stream", "warc": "application/warc", "text": "text/plain", "html": "text/html", "javascript": "text/javascript"}.get(response, "application/json")
    headers = {**target.headers, "Accept": accept}
    if request_body is not None:
        headers["Content-Type"] = "application/json"
    receipt = transport.measure_request(
        method, path, headers=headers, body=request_body,
        timeout=float(target.limits["timeout_seconds"]),
        max_response_bytes=int(target.limits["max_response_bytes"]),
    )
    status, body = receipt.status, receipt.body
    # Even warmup/preflight must not conceal a retry or missing body receipt.
    validate_receipt(receipt)
    if not 200 <= status < 300:
        raise RuntimeError(
            f"{target.label} {path} returned HTTP {status}: {body[:500]!r}"
        )
    if response in GATEWAY_RESPONSES.values():
        validate_gateway(response, body, receipt.headers, target.service_socket)
        return receipt
    if response in {"binary", "warc"}:
        (validate_archive(body, receipt.headers) if response == "warc" else validate_file(path, body, receipt.headers))
        return receipt
    if response == "text":
        assert body.decode("utf-8"), "empty text response"
        return receipt
    try:
        payload = json.loads(body)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"{target.label} {path} returned non-JSON: {body[:500]!r}"
        ) from error
    if path == BODY_URL:
        assert_event_bodies(payload)
    elif "/bodies/" in path:
        validate_full_event(payload)
    return receipt


def service_target(service, label: str) -> Target:
    """Address the existing real service companion, never spawn another."""
    from .stream_measurements import service_gateway
    assert service.proc is not None
    process = Process(service.proc.pid)
    if label == "service":
        return Target(label, lambda: Transport(socket_path=str(service.uds_path)), {}, {"service": process})
    if label != "gateway":
        raise ValueError("unsupported measurement target")
    gateway = service_gateway(service)
    return Target(label, lambda: Transport(host="127.0.0.1", port=gateway.port),
                  {"Authorization": f"Bearer {gateway.token}"},
                  {"gateway": Process(gateway.pid), "downstream_service": process},
                  service_socket=str(service.uds_path))


def json_request(transport, target: Target, method: str, path: str, body, *, timeout: float, maximum: int):
    """Finite mutations reuse the existing strict single-attempt HTTP owner."""
    headers = {**target.headers, "Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    result = transport.measure_request(method, path, headers=headers,
        body=None if body is None else json.dumps(body).encode(),
        timeout=timeout, max_response_bytes=maximum)
    validate_receipt(result)
    if result.status != 200:
        raise ValueError(f"{method} {path} refused with HTTP {result.status}")
    return result, json.loads(result.body)

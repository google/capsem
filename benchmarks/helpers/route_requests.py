"""Typed route request inputs and post-clock validation, using one HTTP owner."""
from __future__ import annotations

import json
import tomllib
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

import psutil
from helpers.http_measurement import MeasuredResponse
from helpers.http_transport import Transport
from helpers.stopped_workspace import BODY_URL, assert_event_bodies

from .route_binary import validate_archive, validate_file, validate_full_event
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
    cpu_processes: dict[str, psutil.Process]
    limits: dict[str, int | float] = field(default_factory=route_load_config)


def request(transport: Transport, target: Target, path: str, response: str = "json") -> MeasuredResponse:
    """Require one complete first-attempt JSON/text receipt."""
    method, request_body = request_input(path)
    accept = {"binary": "application/octet-stream", "warc": "application/warc", "text": "text/plain"}.get(response, "application/json")
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

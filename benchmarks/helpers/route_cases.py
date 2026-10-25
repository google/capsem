"""Configured method/path cases against the existing stopped and live fixtures."""
from __future__ import annotations

import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlencode

from helpers.stopped_workspace import BODY_ROUTE, BODY_URL, VM_ID

from .route_gateway import GATEWAY_RESPONSES

ROOT = Path(__file__).resolve().parents[2]
LIVE_ROUTES = {
    "/vms/{id}/exposures", "/vms/{id}/security/status",
    "/vms/{id}/container", "/vms/{id}/security/asks", "/vms/{id}/files/content",
}
PLUGIN_CASES = {"credential_broker", "pii_filter", "secret_exfiltration", "utf8_guard"}
MANAGED_REQUEST_ID = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61"


@dataclass(frozen=True)
class RouteCase:
    method: str
    path: str
    case: str
    parameters: dict[str, str] = field(default_factory=dict)
    query: dict[str, str | int] = field(default_factory=dict)
    response: str = "json"
    target_scope: str = "both"
    samples: int | None = None
    load_requests: int | None = None
    concurrency: int | None = None

    def __post_init__(self) -> None:
        if self.target_scope not in {"both", "gateway"}:
            raise ValueError("unknown route target scope")
        if self.path in GATEWAY_RESPONSES and (self.target_scope != "gateway" or self.response != GATEWAY_RESPONSES[self.path]):
            raise ValueError("gateway-only operation requires explicit gateway scope and response contract")

    def supports_target(self, label: str) -> bool:
        return self.target_scope == "both" or label == "gateway"

    def counts(self, samples: int, load_requests: int, concurrency: int, override: int | None = None) -> tuple[int, int, int]:
        return (override if override is not None else self.samples or samples,
                self.load_requests or load_requests, self.concurrency or concurrency)

    @property
    def live(self) -> bool:
        return self.path in LIVE_ROUTES

    @property
    def metric_key(self) -> str:
        return f"{self.method} {self.path} [{self.case}]"

    def resolve(self, live_id: str | None, network_id: str | None = None) -> str:
        if self.live and live_id is None:
            raise ValueError(f"case requires a live owner fixture: {self.metric_key}")
        if self.path == BODY_ROUTE:
            return BODY_URL if not self.query else BODY_URL.split("?", 1)[0] + "?" + urlencode(self.query)
        if self.path.startswith("/networks/{id}") and network_id is None:
            raise ValueError("case requires a real network fixture")
        subject = network_id if self.path.startswith("/networks/{id}") else live_id if self.live else VM_ID
        assert subject is not None
        path = self.path.replace("{id}", subject)
        path = path.replace("{request_id}", MANAGED_REQUEST_ID)
        if "{plugin_id}" in path and self.case in PLUGIN_CASES:
            path = path.replace("{plugin_id}", self.case)
        for name, value in self.parameters.items():
            path = path.replace("{" + name + "}", value)
        if "{" in path or "}" in path:
            raise ValueError(f"case has no successful fixture: {self.metric_key}")
        return path + ("?" + urlencode(self.query) if self.query else "")


def measured_cases() -> list[RouteCase]:
    config = tomllib.loads((ROOT / "config/gate.toml").read_text())
    return [RouteCase(**case) for case in config["benchmark"]["routes"]["measured"]]

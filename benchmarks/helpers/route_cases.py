"""Configured method/path cases against the existing stopped and live fixtures."""
from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path

from helpers.stopped_workspace import BODY_ROUTE, BODY_URL, VM_ID

ROOT = Path(__file__).resolve().parents[2]
LIVE_ROUTES = {
    "/vms/{id}/exposures", "/vms/{id}/security/status",
    "/vms/{id}/container", "/vms/{id}/security/asks",
}
PLUGIN_CASES = {"credential_broker", "pii_filter", "secret_exfiltration", "utf8_guard"}
MANAGED_REQUEST_ID = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61"


@dataclass(frozen=True)
class RouteCase:
    method: str
    path: str
    case: str

    @property
    def live(self) -> bool:
        return self.path in LIVE_ROUTES

    @property
    def metric_key(self) -> str:
        return f"{self.method} {self.path} [{self.case}]"

    def resolve(self, live_id: str | None) -> str:
        if self.live and live_id is None:
            raise ValueError(f"case requires a live owner fixture: {self.metric_key}")
        if self.path == BODY_ROUTE:
            return BODY_URL
        subject = live_id if self.live else VM_ID
        assert subject is not None
        path = self.path.replace("{id}", subject)
        path = path.replace("{request_id}", MANAGED_REQUEST_ID)
        if "{plugin_id}" in path and self.case in PLUGIN_CASES:
            path = path.replace("{plugin_id}", self.case)
        if "{" in path or "}" in path:
            raise ValueError(f"case has no successful fixture: {self.metric_key}")
        return path


def measured_cases() -> list[RouteCase]:
    config = tomllib.loads((ROOT / "config/gate.toml").read_text())
    return [RouteCase(**case) for case in config["benchmark"]["routes"]["measured"]]

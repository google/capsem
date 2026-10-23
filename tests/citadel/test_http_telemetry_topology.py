"""Normalized labels must cover registered templates without admitting raw URIs."""
import re
from pathlib import Path

from helpers.route_inventory import function_operations, route_operations

ROOT = Path(__file__).parents[2]
RATIONALE = "HTTP telemetry admits a finite catalogue of actual route templates, never raw paths; update the label owner with the router."


def test_metric_route_labels_match_real_service_and_gateway_topology() -> None:
    service = ROOT / "crates/capsem-service/src/router_runtime.rs"
    gateway = ROOT / "crates/capsem-gateway/src/main.rs"
    operations = function_operations(service, "fn build_service_router") | route_operations(gateway.read_text())
    for filename in ("schema.rs", "surface.rs"):
        operations |= function_operations(gateway.parent / filename, "fn routes()")
    source = (ROOT / "crates/capsem-telemetry/src/http/operations.rs").read_text()
    admitted = re.findall(r'^    "([^"\n]+)",$', source, re.M)
    assert admitted == sorted(set(admitted)), RATIONALE
    assert set(admitted) == {entry.split(" ", 1)[1] for entry in operations}, RATIONALE

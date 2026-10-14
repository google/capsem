"""The route collector uses real stopped-session URLs and refuses unseeded paths."""

import sys
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
loader = SourceFileLoader("benchmark_routes", str(ROOT / "benchmarks/collectors/routes"))
spec = spec_from_loader(loader.name, loader)
assert spec is not None
collector = module_from_spec(spec)
sys.modules[loader.name] = collector
loader.exec_module(collector)


@pytest.mark.parametrize(
    ("pattern", "expected"),
    [
        ("/connections", "/connections"),
        ("/status", "/status"),
        ("/vms/{id}/status", f"/vms/{collector.VM_ID}/status"),
        ("/vms/{id}/credential-grants", f"/vms/{collector.VM_ID}/credential-grants"),
        (collector.BODY_ROUTE, collector.BODY_URL),
    ],
)
def test_measured_paths_use_the_actual_fixture(pattern: str, expected: str) -> None:
    assert collector.request_path(pattern) == expected


@pytest.mark.parametrize("pattern", ["/connections/{connection_id}", "/vms/{id}/bodies/{other}"])
def test_a_route_without_a_successful_fixture_is_refused(pattern: str) -> None:
    with pytest.raises(ValueError, match="fixture"):
        collector.request_path(pattern)

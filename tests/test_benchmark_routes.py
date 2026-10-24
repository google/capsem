"""The route collector resolves real fixture URLs and refuses unseeded paths."""

import json
import sys
import tomllib
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
from pathlib import Path
from types import SimpleNamespace

import pytest
from helpers.stopped_workspace import BODY_ROUTE, BODY_URL, VM_ID

from benchmarks.helpers.route_measurements import Observations

ROOT = Path(__file__).resolve().parents[1]
config = tomllib.loads((ROOT / "config/gate.toml").read_text())
loader = SourceFileLoader("benchmark_routes", str(ROOT / config["benchmark"]["run"]["collectors"] / "routes"))
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
        ("/vms/{id}/status", f"/vms/{VM_ID}/status"),
        ("/vms/{id}/credential-grants", f"/vms/{VM_ID}/credential-grants"),
        (BODY_ROUTE, BODY_URL),
    ],
)
def test_measured_paths_use_the_actual_fixture(pattern: str, expected: str) -> None:
    assert collector.RouteCase("GET", pattern, "default").resolve(None) == expected


@pytest.mark.parametrize("pattern", ["/connections/{connection_id}", "/vms/{id}/bodies/{other}"])
def test_a_route_without_a_successful_fixture_is_refused(pattern: str) -> None:
    with pytest.raises(ValueError, match="fixture"):
        collector.RouteCase("GET", pattern, "default").resolve(None)


@pytest.mark.parametrize("pattern", ["/vms/{id}/exposures", "/vms/{id}/security/status"])
def test_live_owner_routes_require_the_live_fixture(pattern: str) -> None:
    with pytest.raises(ValueError, match="live"):
        collector.RouteCase("GET", pattern, "default").resolve(None)
    live_id = "2ac847f8-55fa-46e6-93f4-bd91db45d4ea"
    assert collector.RouteCase("GET", pattern, "default").resolve(live_id) == pattern.replace("{id}", live_id)
    assert collector.RouteCase("GET", BODY_ROUTE, "default").resolve(live_id) == BODY_URL


def test_fixture_diagnostics_do_not_corrupt_collector_json(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    tmp_path: Path,
) -> None:
    stopped = []
    service = SimpleNamespace(
        tmp_dir=tmp_path,
        home_dir=tmp_path,
        uds_path=tmp_path / "service.sock",
        proc=SimpleNamespace(pid=1),
        start=lambda: print("SERVICE LOG: isolated fixture"),
        stop=lambda: stopped.append("service"),
    )
    gateway = SimpleNamespace(
        port=12345,
        token="fixture-token",
        pid=2,
        start=lambda: print("GATEWAY LOG: isolated fixture"),
        stop=lambda: stopped.append("gateway"),
    )
    monkeypatch.setattr(collector, "ServiceInstance", lambda: service)
    monkeypatch.setattr(collector, "service_gateway", lambda _: gateway)
    monkeypatch.setattr(collector, "seed_stopped_workspace", lambda *_, **__: None)
    monkeypatch.setattr(collector, "clone_benchmark_assets", lambda *_: tmp_path / "benchmark-assets")
    original_process = collector.psutil.Process
    monkeypatch.setattr(collector.psutil, "Process", lambda _: None)
    monkeypatch.setattr(collector, "measured_cases", lambda: [collector.RouteCase("GET", "/status", "default")])
    monkeypatch.setattr(collector, "collect_policy", lambda *_: {})
    monkeypatch.setattr(collector, "collect_lifecycle", lambda *_: {})
    monkeypatch.setattr(collector, "collect_networks", lambda *_: {})
    monkeypatch.setattr(collector, "collect_proxies", lambda *_: {})
    monkeypatch.setattr(collector, "collect_workload_controls", lambda *_: {})
    monkeypatch.setattr(collector, "collect_connection_errors", lambda *_: {})
    monkeypatch.setattr(collector, "collect_network_members", lambda *_: {})
    monkeypatch.setattr(collector, "collect_images", lambda *_: (print("IMAGE PREPARATION: untimed") or {}))
    monkeypatch.setattr(collector, "preflight", lambda *_: None)
    monkeypatch.setattr(collector, "seed_metadata", lambda *_: None)
    monkeypatch.setattr(collector, "seed_logs", lambda *_: None)
    monkeypatch.setattr(
        collector, "sample_warm_route",
        lambda target, *_: (Observations([0.1], [0.01], [0.02]), dict.fromkeys(target.cpu_processes, 0.0)),
    )
    monkeypatch.setattr(collector, "sample_cold_route", lambda *_: Observations([0.2], [0.01], [0.02]))
    monkeypatch.setattr(
        collector, "sample_concurrent_route",
        lambda target, path, samples, *_: (Observations([0.3] * samples, [0.01] * samples, [0.02] * samples), dict.fromkeys(target.cpu_processes, 0.0), 10.0),
    )
    monkeypatch.setattr(sys, "argv", ["routes", "--samples", "1"])

    try:
        assert collector.main() == 0
    finally:
        # psutil is shared with the process-leak fixture, which tears down
        # before monkeypatch's automatic undo.
        monkeypatch.setattr(collector.psutil, "Process", original_process)
    captured = capsys.readouterr()
    payload = json.loads(captured.out)
    assert len(payload["metrics"]) == 26
    assert "SERVICE LOG:" in captured.err
    assert stopped == ["service"]


def test_missing_prepared_image_inputs_fail_before_product_children(monkeypatch) -> None:
    created = []
    def unavailable(*_):
        raise ValueError('missing exact retained image input')
    monkeypatch.setattr(collector, 'collect_images', unavailable)
    monkeypatch.setattr(collector, 'ServiceInstance', lambda: created.append(True))
    monkeypatch.setattr(sys, 'argv', ['routes', '--samples', '1'])
    with pytest.raises(ValueError, match='retained image'):
        collector.main()
    assert created == []

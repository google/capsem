"""Gate tool adapters use policy-owned keyed cache stages."""

from pathlib import Path

from capsem_builder import gatelaunch
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.telemetry import ReuseScope
from capsem_builder.gate import cachetooling
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)


def test_environment_selects_uv_generation_and_shared_pnpm_store(monkeypatch) -> None:
    monkeypatch.delenv(CONFIG.environment.source_checkout, raising=False)
    monkeypatch.setattr(cachetooling.shutil, "which", lambda _: "/usr/bin/sccache")
    observed: list[tuple[str, ReuseScope, Path | None, tuple[str, ...]]] = []

    def observe(
        _paths: CachePaths,
        stage_id: str,
        *,
        tool: str,
        key: str,
        scope: ReuseScope,
        observed_bytes: int | None = None,
        probe: Path | None = None,
        ignored_names: tuple[str, ...] = (),
    ) -> None:
        del tool, key, observed_bytes
        observed.append((stage_id, scope, probe, ignored_names))

    monkeypatch.setattr(cachetooling, "record_use", observe)

    environment = cachetooling.environment(CONFIG, key="source")

    assert environment[CONFIG.environment.uv_cache] == str(ROOT / "cache/tools/python/uv")
    assert environment[gatelaunch.RUFF_CACHE] == str(ROOT / "cache/tools/python/ruff")
    assert environment[CONFIG.environment.pnpm_store] == str(ROOT / "cache/tools/node/pnpm")
    assert Path(environment[cachetooling.PYTHONPYCACHEPREFIX]).parent == (
        ROOT / "cache/tools/python/pycache"
    )
    assert "cache_dir=" in environment[cachetooling.PYTEST_ADDOPTS]
    assert str(ROOT / "cache/tools/python/pytest") in environment[cachetooling.PYTEST_ADDOPTS]
    compiler = cachetooling.compiler_environment(CONFIG)
    assert compiler[CONFIG.environment.rustc_wrapper] == "sccache"
    assert compiler[CONFIG.environment.sccache_dir] == str(ROOT / "cache/tools/rust/sccache")
    assert compiler[CONFIG.environment.sccache_cache_size] == "200G"
    assert compiler[CONFIG.environment.sccache_base_dir] == str(ROOT)
    assert compiler[CONFIG.environment.sccache_server_uds] == str(
        ROOT / "cache/tools/rust/sccache/sccache.sock"
    )
    by_stage = {stage_id: (scope, probe, ignored) for stage_id, scope, probe, ignored in observed}
    assert [stage_id for stage_id, *_ in observed] == [
        "python-uv",
        "python-ruff",
        "python-pycache",
        "python-pytest",
        "node-pnpm",
        "rust-sccache",
    ]
    assert by_stage["python-uv"][0] is ReuseScope.SHARED
    assert by_stage["python-ruff"][0] is ReuseScope.SHARED
    assert by_stage["python-pycache"][0] is ReuseScope.GENERATION
    assert by_stage["python-pytest"][0] is ReuseScope.GENERATION
    python_probe = by_stage["python-pycache"][1]
    pytest_probe = by_stage["python-pytest"][1]
    assert python_probe is not None
    assert pytest_probe is not None
    assert python_probe.parent == ROOT / "cache/tools/python/pycache"
    assert pytest_probe.parent == ROOT / "cache/tools/python/pytest"
    assert by_stage["node-pnpm"][0] is ReuseScope.SHARED
    assert by_stage["rust-sccache"][0] is ReuseScope.SHARED
    assert by_stage["rust-sccache"][2] == ("sccache.sock",)


def test_missing_compiler_cache_keeps_cargo_on_its_first_bootstrap(monkeypatch, tmp_path) -> None:
    monkeypatch.setattr(cachetooling.shutil, "which", lambda _: None)
    monkeypatch.setattr(cachetooling, "record_use", lambda *args, **kwargs: None)
    config = CONFIG.model_copy(update={"root": tmp_path})

    environment = cachetooling.compiler_environment(config)

    assert CONFIG.environment.rustc_wrapper not in environment


def test_dependency_free_bootstrap_matches_the_typed_tool_selection(monkeypatch) -> None:
    monkeypatch.delenv(CONFIG.environment.source_checkout, raising=False)
    monkeypatch.setattr(cachetooling, "record_use", lambda *args, **kwargs: None)

    typed = cachetooling.environment(CONFIG, key="parity")
    bootstrap = gatelaunch.contained_environment(ROOT)

    assert bootstrap[gatelaunch.PYCACHE] == typed[cachetooling.PYTHONPYCACHEPREFIX]
    assert bootstrap[gatelaunch.PYTEST_ADDOPTS] == typed[cachetooling.PYTEST_ADDOPTS]
    assert bootstrap[gatelaunch.UV_CACHE] == typed[CONFIG.environment.uv_cache]
    assert bootstrap[gatelaunch.RUFF_CACHE] == typed[gatelaunch.RUFF_CACHE]
    assert bootstrap[gatelaunch.PNPM_STORE] == typed[CONFIG.environment.pnpm_store]

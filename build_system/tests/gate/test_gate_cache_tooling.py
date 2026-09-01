"""Gate tool adapters use policy-owned keyed cache stages."""

from pathlib import Path

from capsem_builder.gate import cachetooling
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)


def test_environment_selects_uv_generation_and_shared_pnpm_store(monkeypatch) -> None:
    monkeypatch.delenv(CONFIG.environment.source_checkout, raising=False)
    monkeypatch.setattr(cachetooling.shutil, "which", lambda _: "/usr/bin/sccache")
    observed: list[tuple[str, str]] = []
    monkeypatch.setattr(
        cachetooling,
        "record_use",
        lambda paths, stage_id, *, tool, key, **kwargs: observed.append((stage_id, tool)),
    )

    environment = cachetooling.environment(CONFIG, key="source")

    assert Path(environment[CONFIG.environment.uv_cache]).parent == ROOT / "cache/tools/python/uv"
    assert environment[CONFIG.environment.pnpm_store] == str(ROOT / "cache/tools/node/pnpm")
    compiler = cachetooling.compiler_environment(CONFIG)
    assert compiler[CONFIG.environment.rustc_wrapper] == "sccache"
    assert compiler[CONFIG.environment.sccache_dir] == str(ROOT / "cache/tools/rust/sccache")
    assert compiler[CONFIG.environment.sccache_cache_size] == "32G"
    assert compiler[CONFIG.environment.sccache_base_dir] == str(ROOT)
    assert compiler[CONFIG.environment.sccache_server_uds] == str(
        ROOT / "cache/tools/rust/sccache/sccache.sock"
    )
    assert observed == [
        ("python-uv", "uv"),
        ("python-pycache", "python"),
        ("node-pnpm", "pnpm"),
        ("rust-sccache", "sccache"),
    ]


def test_missing_compiler_cache_keeps_cargo_on_its_first_bootstrap(monkeypatch, tmp_path) -> None:
    monkeypatch.setattr(cachetooling.shutil, "which", lambda _: None)
    monkeypatch.setattr(cachetooling, "record_use", lambda *args, **kwargs: None)
    config = CONFIG.model_copy(update={"root": tmp_path})

    environment = cachetooling.compiler_environment(config)

    assert CONFIG.environment.rustc_wrapper not in environment

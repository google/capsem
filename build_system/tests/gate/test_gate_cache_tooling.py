"""Gate tool adapters use policy-owned keyed cache stages."""

from pathlib import Path

from capsem_builder.gate import cachetooling
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)


def test_environment_selects_uv_generation_and_shared_pnpm_store(monkeypatch) -> None:
    observed: list[tuple[str, str]] = []
    monkeypatch.setattr(
        cachetooling,
        "record_use",
        lambda paths, stage_id, *, tool, key: observed.append((stage_id, tool)),
    )

    environment = cachetooling.environment(CONFIG, key="source")

    assert Path(environment[CONFIG.environment.uv_cache]).parent == ROOT / "cache/tools/python/uv"
    assert environment[CONFIG.environment.pnpm_store] == str(ROOT / "cache/tools/node/pnpm")
    assert observed == [
        ("python-uv", "uv"),
        ("python-pycache", "python"),
        ("node-pnpm", "pnpm"),
    ]

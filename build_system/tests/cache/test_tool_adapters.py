"""Native tools resolve to the same policy-owned cache stages."""

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def test_every_pnpm_workspace_resolves_the_owned_store() -> None:
    workspaces = ("web/app", "web/docs", "web/marketing", "build_system/release_site")
    expected = ROOT / "cache/tools/node/pnpm"

    for workspace in workspaces:
        config = ROOT / workspace / ".npmrc"
        value = config.read_text(encoding="utf-8").strip().removeprefix("store-dir=")
        assert (config.parent / value).resolve() == expected


def test_cargo_retention_selects_incremental_state_under_native_locks() -> None:
    policy = tomllib.loads((ROOT / "config/cache.toml").read_text(encoding="utf-8"))

    cargo = policy["stages"]["cargo"]
    assert cargo["prune_strategy"] == "generational"
    assert cargo["retention_root"] == "debug/incremental"
    assert "debug/.cargo-lock" in cargo["mutation_locks"]


def test_bootstrap_and_gate_share_one_uv_content_store() -> None:
    policy = tomllib.loads((ROOT / "config/cache.toml").read_text(encoding="utf-8"))
    configured = tomllib.loads((ROOT / "uv.toml").read_text(encoding="utf-8"))["cache-dir"]
    expected = ROOT / policy["root"] / policy["stages"]["python-uv"]["path"]

    assert (ROOT / configured).resolve() == expected

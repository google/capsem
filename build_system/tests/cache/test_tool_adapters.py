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


def test_cargo_live_profiles_are_observed_but_not_selectively_pruned() -> None:
    policy = tomllib.loads((ROOT / "config/cache.toml").read_text(encoding="utf-8"))

    assert policy["stages"]["cargo"]["prune"] == "none"


def test_bootstrap_uv_cache_is_inside_the_owned_stage() -> None:
    configured = tomllib.loads((ROOT / "uv.toml").read_text(encoding="utf-8"))["cache-dir"]

    assert (ROOT / configured).is_relative_to(ROOT / "cache/tools/python/uv")

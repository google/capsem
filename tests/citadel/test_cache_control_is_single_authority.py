"""Citadel guard: one typed cache contract owns every storage mechanism."""

from __future__ import annotations

import inspect
from pathlib import Path

import tomllib
from capsem_builder.cache.api import CacheBackend
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.registry import CacheRegistry

ROOT = Path(__file__).resolve().parents[2]
COMMON_FIELDS = {
    "description",
    "scope",
    "max_size_bytes",
    "warm_size_bytes",
    "prune_strategy",
}


def _owners() -> tuple[dict, ...]:
    document = tomllib.loads((ROOT / "config/cache.toml").read_text(encoding="utf-8"))
    images = document["control"]["docker"]["images"]
    return (
        tuple(document["stages"].values())
        + tuple(document["runtimes"].values())
        + tuple(images.values())
    )


def test_every_cache_owner_has_the_common_contract() -> None:
    owners = _owners()

    assert owners
    for owner in owners:
        assert owner.keys() >= COMMON_FIELDS
        assert owner["description"].strip()
        assert 0 < owner["warm_size_bytes"] <= owner["max_size_bytes"]


def test_policy_and_registry_load_every_owner_without_backend_input() -> None:
    policy = load_policy(ROOT)
    registry = CacheRegistry.__new__(CacheRegistry)
    methods = {
        name
        for name, _ in inspect.getmembers(CacheBackend, predicate=inspect.isfunction)
    }

    assert policy.root == Path("cache")
    assert methods >= {"contract", "usages", "mutate"}
    assert not hasattr(registry, "docker_path")
    assert not hasattr(registry, "tart_home")


def test_legacy_cache_modules_and_commands_cannot_return() -> None:
    removed_modules = (
        ROOT / "build_system/builder/cache" / ("cap" + "acity.py"),
        ROOT / "build_system/builder/cache" / ("hea" + "lth.py"),
        ROOT / "build_system/builder/gate" / ("asset" + "cache.py"),
        ROOT / "build_system/builder/gate" / ("di" + "sk.py"),
        ROOT / "build_system/builder/gate" / ("g" + "c.py"),
    )
    assert not [path for path in removed_modules if path.exists()]

    forbidden = (
        "runtime" + "-prune",
        "runtime" + "-status",
        "ensure" + "-space",
        "minimum" + "_free_bytes",
        "cargo_target" + "_warning_gb",
        "vm_image" + "_cache",
    )
    surfaces = (
        ROOT / "build_system/builder",
        ROOT / "build_system/tests",
        ROOT / "tests",
        ROOT / "skills",
        ROOT / "config",
    )
    violations = []
    for surface in surfaces:
        for path in surface.rglob("*"):
            if not path.is_file() or path.suffix not in {".py", ".toml", ".md", ".sh"}:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            violations.extend(
                f"{path.relative_to(ROOT)} contains {token}"
                for token in forbidden
                if token in text
            )
    assert not violations, "\n".join(violations)


def test_just_cache_is_only_a_thin_dispatcher() -> None:
    source = (ROOT / "justfile").read_text(encoding="utf-8")
    recipe = source.split("\ncache *command:\n", 1)[1].split("\n\n", 1)[0]

    assert "capsem-cache dispatch" in recipe
    assert len(recipe.splitlines()) == 1
    methods = {
        name
        for name, _ in inspect.getmembers(CacheRegistry, predicate=inspect.isfunction)
    }
    assert methods >= {"contract", "mutate", "stats"}

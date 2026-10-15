"""Runtime cache mechanics and cache-provider dependencies have one owner."""

from __future__ import annotations

import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CACHE_PROVIDERS = frozenset({"quick_cache", "lru", "moka", "cached"})

RATIONALE = """\
Runtime cache mechanics have one owner: capsem-cache.

Adding an eviction/cache crate or calling a provider directly from a product
crate recreates the split cache system this guard retired: inconsistent bounds,
single-flight behavior, counters, and invalidation. Extend capsem-cache, or use
CacheMetrics when a database or async domain must retain specialized locking.
"""


def dependency_sections(manifest: dict) -> list[dict]:
    sections = [manifest.get("dependencies", {}), manifest.get("dev-dependencies", {})]
    for target in manifest.get("target", {}).values():
        sections.extend((target.get("dependencies", {}), target.get("dev-dependencies", {})))
    return sections


def test_only_capsem_cache_depends_on_a_cache_provider() -> None:
    offenders = []
    for path in sorted((PROJECT_ROOT / "crates").glob("*/Cargo.toml")):
        manifest = tomllib.loads(path.read_text())
        providers = sorted(
            CACHE_PROVIDERS.intersection(
                dependency
                for section in dependency_sections(manifest)
                for dependency in section
            )
        )
        if providers and path.parent.name != "capsem-cache":
            offenders.append(f"{path.relative_to(PROJECT_ROOT)}: {', '.join(providers)}")

    assert not offenders, RATIONALE + "\nDirect provider dependencies:\n" + "\n".join(offenders)


def test_product_sources_do_not_bypass_capsem_cache() -> None:
    provider_paths = ("quick_cache::", "lru::LruCache", "moka::", "cached::")
    offenders = []
    for path in sorted((PROJECT_ROOT / "crates").glob("*/src/**/*.rs")):
        if path.parts[-3] == "capsem-cache":
            continue
        source = path.read_text()
        found = [provider for provider in provider_paths if provider in source]
        if found:
            offenders.append(f"{path.relative_to(PROJECT_ROOT)}: {', '.join(found)}")

    assert not offenders, RATIONALE + "\nDirect provider calls:\n" + "\n".join(offenders)


def test_runtime_cache_owners_use_the_shared_control_plane() -> None:
    required = {
        "crates/capsem-core/src/net/dns/cache.rs": "BoundedCache",
        "crates/capsem-core/src/net/cert_authority.rs": "BoundedCache",
        "crates/capsem-logger/src/db.rs": "KeyedIdentityCache",
        "crates/capsem-service/src/main.rs": "GenerationCache",
        "crates/capsem-service/src/container_setup/images.rs": "CacheMetrics",
    }
    missing = [
        f"{path}: {symbol}"
        for path, symbol in required.items()
        if symbol not in (PROJECT_ROOT / path).read_text()
    ]

    assert not missing, RATIONALE + "\nMissing shared cache owners:\n" + "\n".join(missing)

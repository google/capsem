"""Citadel guard: runtime crates link no native codec, and keep two write rails.

The one C library the runtime ships is SQLite through bundled rusqlite. The
ledger is parsed from attacker-influenced bytes, so every additional native
decoder is attack surface the Rust type system does not cover. Pure-Rust
codecs (miniz_oxide, lz4_flex, ruzstd) are allowed; their *-sys twins are not.

The second rule here is about storage rather than parsing, and it catches what
the first cannot: sled, redb and fjall are pure Rust, so nothing about them
looks wrong. The runtime has two write rails -- SQLite for the ledger,
capsem-archive for the append-only body file -- and a third is a durability
model, not a dependency. Extend capsem-archive instead.
"""

from __future__ import annotations

import tomllib
from collections.abc import Iterable
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
LOCKFILE = PROJECT_ROOT / "Cargo.lock"
WORKSPACE = PROJECT_ROOT / "Cargo.toml"

NATIVE_DEPENDENCY_RATIONALE = """\
Native compression/storage dependency in a runtime crate.

Runtime crates ship exactly one C library: SQLite via bundled rusqlite.
zstd-sys, lz4-sys, librocksdb-sys, libsqlite3-sys outside rusqlite, snappy,
brotli-sys and friends add a native decoder in front of attacker-influenced
ledger bytes. Use a pure-Rust codec (miniz_oxide is the house choice) or move
the work to capsem-admin, which is tooling and not in the runtime. The
dependency closure below is taken over Cargo.lock's resolved graph, which
flattens normal, dev, build, optional and target-specific dependencies
together; scanning that union over-approximates rather than under-approximates,
which is the safe side for a security guard.
See the ledger-archive PR (branch worktree-ledger-archive) for the diagnosis.
"""

RESOLUTION_RATIONALE = """\
Workspace member does not resolve to a lockfile package.

Every crate listed under [workspace.members] in Cargo.toml must have its
[package].name present in Cargo.lock. A member that silently fails to
resolve would silently drop out of the native-dependency scan above --
that must fail loudly here instead of being skipped there.
"""

# libsqlite3-sys is deliberately absent: bundled rusqlite reaches it
# unconditionally, and it is the one native library the runtime is allowed.
FORBIDDEN = {
    "zstd-sys",
    "zstd-safe",
    "zstd",
    "lz4-sys",
    "librocksdb-sys",
    "rocksdb",
    "brotli-sys",
    "bzip2-sys",
    "libz-sys",
    "libz-ng-sys",
    "cloudflare-zlib-sys",
    "lzma-sys",
    "xz2",
    "liblzma-sys",
    "libduckdb-sys",
    "leveldb-sys",
    "lmdb-rkv-sys",
    "libdeflate-sys",
}

# Tooling crates that never ship in the runtime and may use native codecs.
TOOLING_CRATES = {"capsem-admin", "capsem-bench", "capsem-mock-server"}

# Pure-Rust embedded stores. Nothing native about them, and that is exactly why
# they would get in: the guard above would not object, and neither would a
# reviewer looking for a `-sys` crate.
PURE_RUST_STORES = {"sled", "redb", "fjall"}

SECOND_STORE_RATIONALE = """\
A second embedded store in a runtime crate.

sled, redb and fjall are pure Rust, so nothing above objects to them -- which
is the problem. The runtime already has two write rails and they are enough:
SQLite owns the ledger, and capsem-archive owns the append-only body file that
exists because SQLite is the wrong shape for multi-megabyte blobs.

A third rail is not a dependency, it is a durability model. It brings its own
crash semantics, its own fsync story, its own corruption modes and its own
retention question, and every ledger invariant that reads across rails has to
be restated over one more of them. It also has to be recovered by whoever is
holding the incident.

Extend capsem-archive. Its format is one file, crates/capsem-archive/src/
format.rs, and adding a block kind to it is a smaller change than any of the
above. If the archive genuinely cannot express what is needed, that is a
decision to make deliberately and write down, not one to make by adding a
dependency.
"""


def _lock_packages(lock_text: str) -> dict[str, set[str]]:
    """name -> merged direct dependency names, from Cargo.lock.

    A package name can appear more than once with different versions, so
    dependency sets for duplicate names are merged rather than the last one
    winning.
    """
    data = tomllib.loads(lock_text)
    packages: dict[str, set[str]] = {}
    for pkg in data.get("package", []):
        name = pkg["name"]
        dep_names = {dep.split(" ")[0] for dep in pkg.get("dependencies", [])}
        packages.setdefault(name, set()).update(dep_names)
    return packages


def _member_package_names(workspace_manifest_text: str) -> dict[str, str]:
    """workspace member path -> its own [package].name (never the basename)."""
    members = tomllib.loads(workspace_manifest_text)["workspace"]["members"]
    result: dict[str, str] = {}
    for member in members:
        crate_manifest = PROJECT_ROOT / member / "Cargo.toml"
        crate_data = tomllib.loads(crate_manifest.read_text())
        result[member] = crate_data["package"]["name"]
    return result


def _closure(packages: dict[str, set[str]], root: str) -> set[str]:
    seen: set[str] = set()
    stack = [root]
    while stack:
        name = stack.pop()
        if name in seen:
            continue
        seen.add(name)
        stack.extend(packages.get(name, ()))
    return seen


def _inject_dependency(lock_text: str, package: str, dependency: str) -> str:
    """Return lock_text with `dependency` added to `package`'s dependencies.

    Only used to build a synthetic adversarial case; production parsing goes
    through tomllib exclusively.
    """
    marker = f'name = "{package}"'
    start = lock_text.index(marker)
    try:
        end = lock_text.index("\n[[package]]", start)
    except ValueError:
        end = len(lock_text)
    deps_marker = "dependencies = [\n"
    deps_start = lock_text.index(deps_marker, start)
    assert start < deps_start < end, (
        f"{package} has no dependencies block within its own package block; "
        "the synthetic injection would land in the next package instead"
    )
    insertion_point = deps_start + len(deps_marker)
    return (
        lock_text[:insertion_point]
        + f' "{dependency}",\n'
        + lock_text[insertion_point:]
    )


def offenders(lock_text: str, member_names: Iterable[str]) -> list[str]:
    packages = _lock_packages(lock_text)
    result: list[str] = []
    for name in member_names:
        if name in TOOLING_CRATES:
            continue
        hit = sorted(FORBIDDEN & _closure(packages, name))
        if hit:
            result.append(f"{name}: {', '.join(hit)}")
    return result


def test_runtime_crates_link_no_native_compression_or_storage() -> None:
    member_names = _member_package_names(WORKSPACE.read_text()).values()
    found = offenders(LOCKFILE.read_text(), member_names)
    assert not found, "\n".join(found) + "\n" + NATIVE_DEPENDENCY_RATIONALE


def test_every_workspace_member_resolves_to_a_lockfile_package() -> None:
    member_names = _member_package_names(WORKSPACE.read_text()).values()
    packages = _lock_packages(LOCKFILE.read_text())
    missing = sorted(name for name in member_names if name not in packages)
    assert not missing, RESOLUTION_RATIONALE + f"\nUnresolved members: {missing}"


def second_store_offenders(lock_text: str, member_names: Iterable[str]) -> list[str]:
    """Pure predicate: runtime crates whose closure reaches a second store."""
    packages = _lock_packages(lock_text)
    result: list[str] = []
    for name in member_names:
        if name in TOOLING_CRATES:
            continue
        hit = sorted(PURE_RUST_STORES & _closure(packages, name))
        if hit:
            result.append(f"{name}: {', '.join(hit)}")
    return result


def test_no_second_pure_rust_store_without_a_decision() -> None:
    member_names = _member_package_names(WORKSPACE.read_text()).values()
    found = second_store_offenders(LOCKFILE.read_text(), member_names)
    assert not found, "\n".join(found) + "\n" + SECOND_STORE_RATIONALE


def test_second_store_predicate_flags_a_synthetic_store() -> None:
    """Adversarial: no runtime crate uses one today, so the predicate is shown
    against an injected edge rather than against an absence."""
    lock_text = LOCKFILE.read_text()
    injected = _inject_dependency(lock_text, "capsem-logger", "sled 0.34.7")
    found = second_store_offenders(injected, ["capsem-logger"])
    assert found == ["capsem-logger: sled"], found


def test_a_tooling_crate_may_hold_a_store() -> None:
    """The exemption is the same one the native guard uses, for the same
    reason: capsem-admin is not in the runtime and is not recovered at 3am."""
    lock_text = _inject_dependency(LOCKFILE.read_text(), "capsem-logger", "redb 2.0.0")
    assert second_store_offenders(lock_text, ["capsem-admin"]) == []


def test_offenders_flags_a_synthetic_native_dependency() -> None:
    """Adversarial: inject a native codec into a real runtime crate's
    dependency block and confirm the pure predicate names it. This does not
    depend on capsem-admin's live zstd use, which could be refactored away
    without this guard ever having run against a real violation."""
    lock_text = LOCKFILE.read_text()
    injected = _inject_dependency(lock_text, "capsem-logger", "zstd 0.13.3")
    found = offenders(injected, ["capsem-logger"])
    assert any(item.startswith("capsem-logger:") for item in found), found

"""Citadel guard: the Rust workspace keeps its new low-context boundaries.

These rules record the architectural payoff of the Rust workspace split.  A
focused crate that quietly reacquires a heavyweight owner, a thin entrypoint
that grows runtime logic again, or tests buried inline in production source
would all make an individual patch harder to understand and verify.
"""

from __future__ import annotations

import re
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]

FOCUSED_DEPENDENCY_RULES = {
    "capsem-foundation": frozenset(
        {"capsem-core", "capsem-logger", "capsem-assets", "rmcp", "rusqlite"}
    ),
    "capsem-assets": frozenset(
        {"capsem-core", "capsem-logger", "capsem-config", "rmcp", "rusqlite"}
    ),
    "capsem-config": frozenset(
        {"capsem-core", "capsem-logger", "capsem-assets", "rmcp", "rusqlite"}
    ),
    "capsem-credentials": frozenset(
        {"capsem-core", "capsem-logger", "capsem-assets", "rmcp", "rusqlite"}
    ),
    "capsem-mcp-aggregator": frozenset(
        {"capsem-core", "capsem-logger", "capsem-assets", "capsem-config", "rusqlite"}
    ),
}

# These are ceilings, not current line counts.  They leave room for attributes
# and entrypoint-specific comments, but no room for runtime ownership to drift
# back out of the library modules.
THIN_SOURCE_LIMITS = {
    "crates/capsem-app/src/main.rs": 30,
    "crates/capsem-mcp-aggregator/src/main.rs": 20,
    "crates/capsem-credentials/src/lib.rs": 40,
}

INLINE_TEST_BLOCK = re.compile(
    r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*mod\s+tests\s*\{",
    re.MULTILINE,
)

LEGACY_CORE_OWNERS = (
    "asset_manager",
    "manifest_compat",
    "ipc_handshake",
    "log_layer",
    "paths",
    "poll",
    "telemetry",
    "uds",
    "capsem_proto",
)
LEGACY_CORE_OWNER = "|".join(LEGACY_CORE_OWNERS)
DIRECT_LEGACY_CORE_PATH = re.compile(
    rf"\bcapsem_core\s*::\s*(?P<owner>{LEGACY_CORE_OWNER})\b"
)
GROUPED_CORE_IMPORT = re.compile(r"\buse\s+capsem_core\s*::\s*\{(?P<body>[^}}]*)\}", re.DOTALL)
RENAMED_CORE_IMPORT = re.compile(
    r"\b(?:use|extern\s+crate)\s+capsem_core\s+as\s+(?P<alias>[A-Za-z_]\w*)\s*;"
)
EXTRACTED_CRATE_REEXPORT = re.compile(
    r"\bpub\s+use\s+capsem_(?:assets|foundation|proto)\b"
)

BOUNDARY_RATIONALE = """\
The extracted Rust crates must remain dependency-light and independently
testable. Reintroducing capsem-core, the session database, or an unrelated
domain owner defeats the split and restores the compile-time and context cost
it removed. Put shared contracts in capsem-proto/foundation or keep the logic
with its owning crate instead.
"""

SOURCE_SHAPE_RATIONALE = """\
Rust entrypoints stay as wiring and tests stay in sibling tests.rs files.
Growing runtime logic in main.rs or burying fixtures below production code
makes every read larger and prevents agents from loading one concern at a
time. Move runtime code into a library module and test it beside that module.
"""

COMPATIBILITY_RATIONALE = """\
Assets, host plumbing, and wire contracts have named owner crates. Importing
them through capsem-core or publicly reexporting those crates restores the
catch-all dependency boundary and makes the extraction cosmetic. Depend on
capsem-assets, capsem-foundation, or capsem-proto directly.
"""


def _runtime_dependencies(table: Mapping[str, Any]) -> set[str]:
    """Collect normal/build dependencies, including target-specific tables."""
    found: set[str] = set()
    for key, value in table.items():
        if key in {"dependencies", "build-dependencies"} and isinstance(value, Mapping):
            for alias, declaration in value.items():
                package = (
                    declaration.get("package", alias)
                    if isinstance(declaration, Mapping)
                    else alias
                )
                found.add(str(package))
        elif key != "dev-dependencies" and isinstance(value, Mapping):
            found.update(_runtime_dependencies(value))
    return found


def _dependency_violations(
    manifests: Mapping[str, Mapping[str, Any]],
) -> dict[str, list[str]]:
    violations: dict[str, list[str]] = {}
    for crate, forbidden in FOCUSED_DEPENDENCY_RULES.items():
        present = sorted(_runtime_dependencies(manifests[crate]) & forbidden)
        if present:
            violations[crate] = present
    return violations


def _thin_source_violations(sources: Mapping[str, str]) -> dict[str, tuple[int, int]]:
    return {
        path: (len(sources[path].splitlines()), limit)
        for path, limit in THIN_SOURCE_LIMITS.items()
        if len(sources[path].splitlines()) > limit
    }


def _inline_test_blocks(sources: Mapping[str, str]) -> list[str]:
    return sorted(
        path for path, source in sources.items() if INLINE_TEST_BLOCK.search(source)
    )


def _legacy_core_owner_paths(sources: Mapping[str, str]) -> dict[str, list[str]]:
    violations: dict[str, list[str]] = {}
    owner_name = re.compile(rf"\b({LEGACY_CORE_OWNER})\b")
    for path, source in sources.items():
        found = {match.group("owner") for match in DIRECT_LEGACY_CORE_PATH.finditer(source)}
        for grouped in GROUPED_CORE_IMPORT.finditer(source):
            found.update(owner_name.findall(grouped.group("body")))
        for renamed in RENAMED_CORE_IMPORT.finditer(source):
            alias = re.escape(renamed.group("alias"))
            found.update(
                match.group("owner")
                for match in re.finditer(
                    rf"\b{alias}\s*::\s*(?P<owner>{LEGACY_CORE_OWNER})\b",
                    source,
                )
            )
        if found:
            violations[path] = sorted(found)
    return violations


def _extracted_crate_reexports(sources: Mapping[str, str]) -> list[str]:
    return sorted(
        path for path, source in sources.items() if EXTRACTED_CRATE_REEXPORT.search(source)
    )


def _tracked_rust_sources() -> dict[str, str]:
    paths = subprocess.run(
        ["git", "ls-files", "--", "crates/**/*.rs"],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    return {
        path: source.read_text()
        for path in paths
        if (source := PROJECT_ROOT / path).is_file()
    }


def test_focused_crates_do_not_reacquire_heavyweight_owners() -> None:
    import tomllib

    manifests = {
        crate: tomllib.loads(
            (PROJECT_ROOT / "crates" / crate / "Cargo.toml").read_text()
        )
        for crate in FOCUSED_DEPENDENCY_RULES
    }
    violations = _dependency_violations(manifests)
    assert not violations, (
        BOUNDARY_RATIONALE + f"\nForbidden direct dependencies: {violations}"
    )


def test_new_thin_rust_sources_stay_thin() -> None:
    sources = {path: (PROJECT_ROOT / path).read_text() for path in THIN_SOURCE_LIMITS}
    violations = _thin_source_violations(sources)
    assert not violations, (
        SOURCE_SHAPE_RATIONALE + f"\nLine ceilings exceeded: {violations}"
    )


def test_rust_tests_live_in_sibling_files() -> None:
    violations = _inline_test_blocks(_tracked_rust_sources())
    assert not violations, (
        SOURCE_SHAPE_RATIONALE + f"\nInline test modules: {violations}"
    )


def test_consumers_import_extracted_owners_directly() -> None:
    violations = _legacy_core_owner_paths(_tracked_rust_sources())
    assert not violations, (
        COMPATIBILITY_RATIONALE + f"\nLegacy capsem-core paths: {violations}"
    )


def test_core_does_not_reexport_extracted_owner_crates() -> None:
    sources = {
        path: source
        for path, source in _tracked_rust_sources().items()
        if path.startswith("crates/capsem-core/")
    }
    violations = _extracted_crate_reexports(sources)
    assert not violations, (
        COMPATIBILITY_RATIONALE + f"\nExtracted crate reexports: {violations}"
    )


def test_dependency_guard_detects_normal_target_and_renamed_dependencies() -> None:
    manifests: dict[str, dict[str, Any]] = {
        crate: {} for crate in FOCUSED_DEPENDENCY_RULES
    }
    manifests["capsem-foundation"] = {
        "dependencies": {"db": {"package": "rusqlite", "version": "0.1"}},
        "target": {"cfg(unix)": {"dependencies": {"capsem-core": {"path": "../core"}}}},
        "dev-dependencies": {"rmcp": "1"},
    }
    assert _dependency_violations(manifests) == {
        "capsem-foundation": ["capsem-core", "rusqlite"]
    }


@pytest.mark.parametrize(
    ("source", "checker"),
    [
        ("line\n" * 31, _thin_source_violations),
        ("#[cfg(test)]\nmod tests { #[test] fn buried() {} }", _inline_test_blocks),
    ],
)
def test_source_shape_guards_notice_a_regression(source: str, checker) -> None:
    if checker is _thin_source_violations:
        sources = dict.fromkeys(THIN_SOURCE_LIMITS, "")
        sources["crates/capsem-app/src/main.rs"] = source
        assert checker(sources) == {"crates/capsem-app/src/main.rs": (31, 30)}
    else:
        assert checker({"crates/example/src/lib.rs": source}) == [
            "crates/example/src/lib.rs"
        ]


@pytest.mark.parametrize(
    ("source", "owner"),
    [
        ("use capsem_core::paths;", "paths"),
        ("use capsem_core::{VmState, telemetry as logs};", "telemetry"),
        ("use capsem_core as core; fn f() { core::manifest_compat::load(); }", "manifest_compat"),
        ("extern crate capsem_core as old; fn f() { old::capsem_proto::version(); }", "capsem_proto"),
    ],
)
def test_legacy_import_guard_detects_direct_grouped_and_renamed_paths(
    source: str, owner: str
) -> None:
    assert _legacy_core_owner_paths({"crates/consumer/src/lib.rs": source}) == {
        "crates/consumer/src/lib.rs": [owner]
    }


@pytest.mark.parametrize(
    "source",
    [
        "pub use capsem_assets::{asset_manager, manifest_compat};",
        "pub use capsem_foundation as host;",
        "pub use capsem_proto::*;",
    ],
)
def test_reexport_guard_detects_extracted_owner_crates(source: str) -> None:
    assert _extracted_crate_reexports({"crates/capsem-core/src/lib.rs": source}) == [
        "crates/capsem-core/src/lib.rs"
    ]

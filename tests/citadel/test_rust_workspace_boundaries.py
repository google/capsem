"""Citadel guard: the Rust workspace keeps its new low-context boundaries.

These rules record the architectural payoff of the Rust workspace split.  A
focused crate that quietly reacquires a heavyweight owner, a thin entrypoint
that grows runtime logic again, or tests buried inline in production source
would all make an individual patch harder to understand and verify.
"""

from __future__ import annotations

import re
import subprocess
import tomllib
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


def _tracked_rust_sources() -> dict[str, str]:
    paths = subprocess.run(
        ["git", "ls-files", "--", "crates/**/*.rs"],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    return {path: (PROJECT_ROOT / path).read_text() for path in paths}


def test_focused_crates_do_not_reacquire_heavyweight_owners() -> None:
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

"""Citadel guard: reusable and generated state has one repository cache root."""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LEGACY = re.compile(r"(?:~?/)?\.cg(?:-[A-Za-z0-9_-]+)?/|(?<!cache/)target/")
RETIRED_CONTROL = (
    "config/storage-policy.toml",
    "docker-storage-policy.py",
    "docker_storage_policy.py",
    "ensure-docker-space.sh",
)
EXCLUDED = (
    "CHANGELOG.md",
    "build_system/tests/cache/",
    "config/cache.toml",
    "tests/citadel/test_cache_root_boundary.py",
)

RATIONALE = """\
Repository build state has one address: cache/. A root target/ or ambient .cg
path bypasses inventory, caps, cleanup, and private-worktree ownership. Historical
changelog/baseline evidence and cache-policy-relative stage paths are not callers.
"""


def _legacy_records(sources: dict[str, str]) -> tuple[str, ...]:
    return tuple(
        sorted(
            f"{path}:{number}:{line.strip()}"
            for path, text in sources.items()
            if not any(
                path == excluded or path.startswith(excluded) for excluded in EXCLUDED
            )
            if "baselines" not in Path(path).parts
            if not (path.startswith("tests/citadel/") and path.endswith("_debt.toml"))
            for number, line in enumerate(text.splitlines(), 1)
            if LEGACY.search(line)
        )
    )


def _tracked_text() -> dict[str, str]:
    paths = subprocess.run(
        ("git", "ls-files", "-z"), cwd=ROOT, check=True, capture_output=True
    ).stdout.split(b"\0")
    sources: dict[str, str] = {}
    for raw_path in paths:
        if not raw_path:
            continue
        path = raw_path.decode()
        candidate = ROOT / path
        if candidate.is_symlink() or not candidate.is_file():
            continue
        try:
            sources[path] = candidate.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
    return sources


def test_legacy_cache_roots_are_observed_red() -> None:
    records = _legacy_records(
        {
            "one.py": 'Path("target/assets")',
            "two.md": "use ~/.cg-build/deadbeef",
        }
    )

    assert len(records) == 2, RATIONALE


def test_generated_state_has_one_root() -> None:
    records = _legacy_records(_tracked_text())

    assert not records, RATIONALE + "\n" + "\n".join(records)


def test_retired_cache_controllers_cannot_return() -> None:
    sources = _tracked_text()
    records = tuple(
        sorted(
            f"{path}:{token}"
            for path, text in sources.items()
            if path not in {"CHANGELOG.md", "LATEST_RELEASE.md"}
            if path != "tests/citadel/test_cache_root_boundary.py"
            for token in RETIRED_CONTROL
            if token in text
        )
    )

    assert not records, RATIONALE + "\n" + "\n".join(records)


def test_runtime_cache_cleanup_has_one_mutation_boundary() -> None:
    sources = _tracked_text()
    forbidden = (
        '"system", "prune"',
        '"image", "prune"',
        '"container", "prune"',
        '"volume", "prune"',
    )
    records = tuple(
        sorted(
            f"{path}:{token}"
            for path, text in sources.items()
            if path.endswith((".py", ".sh"))
            if not path.startswith(("tests/", "build_system/tests/"))
            if path != "build_system/builder/cache/runtimeoperations.py"
            for token in forbidden
            if token in text
        )
    )

    assert not records, RATIONALE + "\n" + "\n".join(records)


def test_root_cache_is_ignored_without_hiding_source_packages() -> None:
    gitignore = (ROOT / ".gitignore").read_text(encoding="utf-8").splitlines()
    dockerignore = (ROOT / ".dockerignore").read_text(encoding="utf-8").splitlines()

    assert "/cache/" in gitignore and "target/" not in gitignore, RATIONALE
    assert "/cache" in dockerignore, RATIONALE
    assert "**/cache" not in dockerignore, RATIONALE


def test_ordinary_cargo_uses_the_owned_compiler_stage() -> None:
    cargo = (ROOT / ".cargo/config.toml").read_text(encoding="utf-8")

    assert '[build]\ntarget-dir = "cache/target/cargo"' in cargo, RATIONALE

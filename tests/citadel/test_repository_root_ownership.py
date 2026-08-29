"""Hold the repository's top-level ownership contract."""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

import pytest

ROOT = Path(__file__).resolve().parents[2]

ROOT_OWNERSHIP_RATIONALE = """\
The repository layout is a product boundary: every tracked top-level path has
one named functional owner. Unknown roots and compatibility symlinks create
second source trees that lint, tests, CI path filters, packaging, and agents
can silently disagree about. The approved migration is recorded in Sprinty
artifact A006; ignored tool/build state is deliberately outside Git's tracked
surface and must not be mistaken for source.
"""

APPROVED_DIRECTORIES = frozenset(
    {
        ".agents",
        ".cargo",
        ".claude",
        ".codex",
        ".config",
        ".cursor",
        ".gemini",
        ".github",
        "benchmarks",
        "build_system",
        "config",
        "crates",
        "guest",
        "release-site",
        "scripts",
        "sdk",
        "skills",
        "src",
        "target",
        "tests",
        "web",
    }
)

APPROVED_ROOT_FILES = frozenset(
    {
        ".dockerignore",
        ".gitignore",
        "AGENTS.md",
        "CHANGELOG.md",
        "CITATION.cff",
        "CLAUDE.md",
        "CONTRIBUTING.md",
        "Cargo.lock",
        "Cargo.toml",
        "GEMINI.md",
        "LATEST_RELEASE.md",
        "LICENSE",
        "README.md",
        "RELEASE.md",
        "SECURITY.md",
        "bootstrap.sh",
        "codecov.yml",
        "justfile",
        "pyproject.toml",
        "rust-toolchain.toml",
        "test-dev-null.sh",
        "uv.lock",
    }
)

MIGRATING_DIRECTORIES = frozenset(
    {
        "release-site",
        "scripts",
        "src",
    }
)

RETIRED_DIRECTORIES = frozenset(
    {
        "assets",
        "bench",
        "data",
        "dist",
        "packages",
        "security",
        "sprints",
        "test-artifacts",
        "tmp",
    }
)


@dataclass(frozen=True)
class TrackedEntry:
    path: PurePosixPath
    mode: str = "100644"


def _violations(entries: list[TrackedEntry]) -> list[str]:
    if not entries:
        return ["Git reported an empty tracked source surface"]

    violations: list[str] = []
    for entry in entries:
        parts = entry.path.parts
        if not parts or entry.path.is_absolute() or ".." in parts:
            violations.append(f"invalid tracked path: {entry.path}")
            continue

        owner = parts[0]
        if len(parts) == 1:
            if owner in APPROVED_DIRECTORIES:
                kind = "symlink" if entry.mode == "120000" else "file"
                violations.append(
                    f"top-level directory owner is a tracked {kind}, not a real "
                    f"directory: {entry.path}"
                )
            elif owner not in APPROVED_ROOT_FILES:
                violations.append(f"unknown tracked root file: {entry.path}")
            continue

        if owner not in APPROVED_DIRECTORIES:
            violations.append(f"unknown tracked top-level directory: {owner}/")

    return sorted(set(violations))


def _tracked_entries(root: Path = ROOT) -> list[TrackedEntry]:
    output = subprocess.run(
        ("git", "ls-files", "-s", "-z"),
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout
    entries: list[TrackedEntry] = []
    for record in output.split(b"\0"):
        if not record:
            continue
        metadata, raw_path = record.split(b"\t", 1)
        mode = metadata.split(b" ", 1)[0].decode("ascii")
        entries.append(TrackedEntry(PurePosixPath(raw_path.decode("utf-8")), mode))
    return entries


def test_unknown_tracked_root_is_rejected() -> None:
    violations = _violations([TrackedEntry(PurePosixPath("mystery/source.py"))])
    assert violations, ROOT_OWNERSHIP_RATIONALE


def test_migrating_root_cannot_become_a_compatibility_symlink() -> None:
    violations = _violations([TrackedEntry(PurePosixPath("src"), "120000")])
    assert violations, ROOT_OWNERSHIP_RATIONALE


def test_empty_tracked_surface_fails_closed() -> None:
    violations = _violations([])
    assert violations, ROOT_OWNERSHIP_RATIONALE


@pytest.mark.parametrize(
    "path",
    [
        ".github/workflows/ci.yaml",
        ".cargo/config.toml",
        "build_system/pyproject.toml",
        "sdk/README.md",
        "web/app/package.json",
    ],
)
def test_approved_hidden_and_target_roots_are_accepted(path: str) -> None:
    assert not _violations([TrackedEntry(PurePosixPath(path))]), (
        ROOT_OWNERSHIP_RATIONALE
    )


def test_ignored_tool_state_is_outside_the_tracked_surface(tmp_path: Path) -> None:
    subprocess.run(("git", "init", "-q"), cwd=tmp_path, check=True)
    (tmp_path / ".gitignore").write_text(".cache/\n", encoding="utf-8")
    (tmp_path / ".github").mkdir()
    (tmp_path / ".github" / "workflow.yaml").write_text("name: test\n", encoding="utf-8")
    (tmp_path / ".cache").mkdir()
    (tmp_path / ".cache" / "generated.bin").write_bytes(b"generated")
    subprocess.run(
        ("git", "add", ".gitignore", ".github/workflow.yaml"),
        cwd=tmp_path,
        check=True,
    )

    entries = _tracked_entries(tmp_path)

    assert {str(entry.path) for entry in entries} == {
        ".gitignore",
        ".github/workflow.yaml",
    }, ROOT_OWNERSHIP_RATIONALE
    assert not _violations(entries), ROOT_OWNERSHIP_RATIONALE


def test_current_tracked_repository_roots_are_owned() -> None:
    violations = _violations(_tracked_entries())
    assert not violations, ROOT_OWNERSHIP_RATIONALE + "\n" + "\n".join(violations)


def test_retired_repository_roots_are_absent() -> None:
    present = sorted(name for name in RETIRED_DIRECTORIES if (ROOT / name).exists())
    assert not present, ROOT_OWNERSHIP_RATIONALE + "\nretired roots: " + ", ".join(present)

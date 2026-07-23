"""Fail-closed contracts for an internally consistent release cut."""

from pathlib import Path

import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _read_text_exact_case(relative_path: str) -> str:
    """Read a repository file only when every path component has exact case."""
    path = PROJECT_ROOT
    for component in Path(relative_path).parts:
        exact_entries = {entry.name for entry in path.iterdir()}
        assert component in exact_entries, (
            f"repository path component {component!r} does not match exact on-disk case "
            f"below {path}"
        )
        path /= component
    return path.read_text()


def _just_recipe_block(name: str) -> str:
    lines = _read_text_exact_case("justfile").splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(name))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def test_release_contract_rejects_wrong_case_even_on_macos() -> None:
    with pytest.raises(AssertionError, match="does not match exact on-disk case"):
        _read_text_exact_case("Justfile")


def test_version_stamp_refreshes_frozen_lock() -> None:
    stamp = _just_recipe_block("_stamp-version:")

    assert 'pyproject.toml' in stamp
    assert "uv lock --offline" in stamp
    assert stamp.index('pyproject.toml') < stamp.index("uv lock --offline")


def test_checked_in_python_lock_matches_project_version() -> None:
    project_version = next(
        line.split('"', 2)[1]
        for line in (PROJECT_ROOT / "pyproject.toml").read_text().splitlines()
        if line.startswith("version = ")
    )
    lock_lines = (PROJECT_ROOT / "uv.lock").read_text().splitlines()
    package_index = next(
        i
        for i, line in enumerate(lock_lines)
        if line == 'name = "capsem"'
    )
    locked_version = lock_lines[package_index + 1].split('"', 2)[1]

    assert locked_version == project_version


def test_release_commands_are_not_a_parallel_just_surface() -> None:
    justfile = _read_text_exact_case("justfile")

    for retired in ("prepare-release", "qualify-release", "cut-release", "release"):
        assert f"\n{retired}:" not in justfile
        assert f"\n{retired} " not in justfile


def test_release_workflow_verifies_qualification_before_publication() -> None:
    release = _read_text_exact_case(".github/workflows/release.yaml")

    verify = release.index("Verify exact commit passed remote qualification")
    package = release.index("build-app-macos:")
    assert verify < package
    assert "scripts/check-release-qualification.py" in release

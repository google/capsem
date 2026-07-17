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


def test_version_stamp_refreshes_frozen_lock_before_release_cut() -> None:
    stamp = _just_recipe_block("_stamp-version:")
    prepare = _just_recipe_block("prepare-release:")

    assert 'pyproject.toml' in stamp
    assert "uv lock --offline" in stamp
    assert stamp.index('pyproject.toml') < stamp.index("uv lock --offline")
    assert "git add " in prepare
    assert "uv.lock" in prepare.split("git add ", 1)[1].splitlines()[0]


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


def test_release_candidate_is_committed_without_minting_a_tag() -> None:
    prepare = _just_recipe_block("prepare-release:")

    assert "prepare-release: test _stamp-version" in prepare
    assert 'git commit -m "release candidate: v${NEW}"' in prepare
    assert "git tag" not in prepare
    assert "gh workflow run" not in prepare
    assert "gh release" not in prepare


def test_release_tag_is_minted_only_after_exact_head_qualification() -> None:
    cut = _just_recipe_block("cut-release ")

    assert "test" not in cut.splitlines()[0]
    assert "_stamp-version" not in cut
    assert 'SHA=$(git rev-parse HEAD)' in cut
    qualification = 'scripts/check-release-qualification.py --sha "$SHA" --channel "$CHANNEL"'
    assert qualification in cut
    assert cut.index(qualification) < cut.index(
        'git tag "$TAG"'
    )
    assert "git commit" not in cut


def test_remote_qualification_dispatches_the_exact_published_head() -> None:
    qualify = _just_recipe_block("qualify-release ")

    assert 'SHA=$(git rev-parse HEAD)' in qualify
    assert 'test "$(git rev-parse origin/main)" = "$SHA"' in qualify
    assert 'gh workflow run release-qualification.yaml --ref main -f "sha=$SHA" -f "channel=$CHANNEL"' in qualify
    assert 'scripts/check-release-qualification.py --sha "$SHA" --channel "$CHANNEL"' in qualify
    assert 'git tag "$TAG"' not in qualify

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


def test_version_stamp_propagates_cargo_toml_and_refreshes_both_frozen_locks() -> None:
    """Stamping reads the version; it never invents one.

    The recipe used to assemble `1.${RELEASE_MINOR}.$(date +%s)` from a pinned
    minor, so the released version recorded when a build ran rather than what
    changed in it. The version is now a human decision in Cargo.toml and this
    recipe only fans it out, which is why the cohort files are read from, not
    written by, the release.
    """
    stamp = _just_recipe_block("_stamp-version:")
    justfile = _read_text_exact_case("justfile")

    assert "release_minor" not in justfile
    assert "Cargo.toml" in stamp
    assert "cargo update --workspace --offline" in stamp
    assert "pyproject.toml" in stamp
    assert "uv lock --offline" in stamp
    assert stamp.index("Cargo.toml") < stamp.index(
        "cargo update --workspace --offline"
    )
    assert stamp.index("pyproject.toml") < stamp.index("uv lock --offline")


def test_version_stamp_refuses_a_version_that_is_already_tagged() -> None:
    """The only thing forcing a deliberate bump.

    Nothing else stops a second release from reusing a version once the version
    stopped being machine-generated: the cohort would agree, the notes would
    regenerate, and the tag collision would surface far later.
    """
    stamp = _just_recipe_block("_stamp-version:")

    assert 'rev-parse -q --verify "refs/tags/v${VERSION}"' in stamp
    assert "already tagged" in stamp
    assert '^[0-9]+\\.[0-9]+\\.[0-9]+$' in stamp


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


def test_checked_in_rust_lock_matches_every_capsem_workspace_package() -> None:
    project_version = next(
        line.split('"', 2)[1]
        for line in (PROJECT_ROOT / "Cargo.toml").read_text().splitlines()
        if line.startswith("version = ")
    )
    lock = (PROJECT_ROOT / "Cargo.lock").read_text()
    package_blocks = lock.split("[[package]]")
    capsem_versions = {
        version
        for block in package_blocks
        if '\nname = "capsem' in block and "\nsource = " not in block
        for version in [
            next(
                line.split('"', 2)[1]
                for line in block.splitlines()
                if line.startswith("version = ")
            )
        ]
    }

    assert capsem_versions == {project_version}


def test_release_commands_are_not_a_parallel_just_surface() -> None:
    justfile = _read_text_exact_case("justfile")

    retired_commands = (
        "prepare-release",
        "qualify-" + "release",
        "cut-" + "release",
        "release",
    )
    for retired in retired_commands:
        assert f"\n{retired}:" not in justfile
        assert f"\n{retired} " not in justfile
    assert "\nrelease-binaries channel:" in justfile
    assert "\nrelease-profile channel profile:" in justfile


def test_binary_release_recipe_uses_one_adversarial_script() -> None:
    justfile = _read_text_exact_case("justfile")
    script = _read_text_exact_case("scripts/release-binaries.py")

    binary_recipe = justfile.split("\nrelease-binaries channel:", 1)[1].split(
        "\n\n", 1
    )[0]
    assert "scripts/release-binaries.py" in binary_recipe
    assert "_build-kernel" not in binary_recipe
    assert "_build-rootfs" not in binary_recipe
    assert "MUTATED_PATHS" in script
    assert "release preparation write set is invalid" in script
    assert '"push", "--atomic", "origin", "main", tag' in script
    assert '"workflow",\n            "run",\n            "release.yaml"' in script
    assert "release-assets.yaml" not in script

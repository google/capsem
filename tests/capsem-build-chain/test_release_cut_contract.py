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


def _planned(command: str, **args) -> str:
    """What a command's plan would run, rendered.

    Replaces reading a recipe body: the recipes are dispatches now, and the
    sequence these contracts are about lives in the plan.
    """
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand
    from capsem.gate.sourcecommit import SourceCommit

    return (
        GateCommand.registry[command](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(
                dry_run=False,
                graph=False,
                timing=False,
                source_commit=SourceCommit("0" * 40),
                **args,
            ),
        )
        ._describe()
        .describe()
    )


def test_version_stamp_propagates_cargo_toml_and_refreshes_both_frozen_locks() -> None:
    """Stamping reads the version; it never invents one.

    The recipe used to assemble `1.${RELEASE_MINOR}.$(date +%s)` from a pinned
    minor, so the released version recorded when a build ran rather than what
    changed in it. The version is now a human decision in Cargo.toml and this
    recipe only fans it out, which is why the cohort files are read from, not
    written by, the release.
    """
    from capsem.gate import config as gate_config

    justfile = _read_text_exact_case("justfile")
    config = gate_config.load(PROJECT_ROOT)
    stamp = (PROJECT_ROOT / "src" / "capsem" / "gate" / "versions.py").read_text(encoding="utf-8")

    assert "release_minor" not in justfile

    # The one authority, and the cohort it fans out to. Both read from config
    # rather than from a recipe body, so the list is data a person can change
    # without touching the code that walks it.
    assert config.versions.cargo_manifest == "Cargo.toml"
    stamped = {entry.path for entry in config.versions.stamped}
    assert "pyproject.toml" in stamped

    # Each lockfile is refreshed by the tool that owns it, after the
    # substitution rather than before -- otherwise the lock records the version
    # the cohort had a moment ago.
    cargo = '["cargo", "update", "--workspace", "--offline"]'
    uv_lock = '["uv", "lock", "--offline"]'
    assert cargo in stamp
    assert uv_lock in stamp
    assert stamp.index("for stamped in settings.stamped") < stamp.index(cargo)
    assert stamp.index(cargo) < stamp.index(uv_lock)


def test_version_stamp_refuses_a_version_that_is_already_tagged() -> None:
    """The only thing forcing a deliberate bump.

    Nothing else stops a second release from reusing a version once the version
    stopped being machine-generated: the cohort would agree, the notes would
    regenerate, and the tag collision would surface far later.
    """
    from capsem.gate import config as gate_config

    versions = (PROJECT_ROOT / "src" / "capsem" / "gate" / "versions.py").read_text(
        encoding="utf-8"
    )
    config = gate_config.load(PROJECT_ROOT)

    # The tag prefix is config rather than a literal in the check, and the
    # refusal is the code that reads it. This was `git rev-parse -q --verify
    # "refs/tags/v${VERSION}"` in a recipe; the claim is that a version already
    # tagged cannot be stamped again.
    assert config.versions.tag_prefix == "refs/tags/v"
    assert "already tagged" in versions
    assert "tag_prefix" in versions


def test_checked_in_python_lock_matches_project_version() -> None:
    project_version = next(
        line.split('"', 2)[1]
        for line in (PROJECT_ROOT / "pyproject.toml").read_text().splitlines()
        if line.startswith("version = ")
    )
    lock_lines = (PROJECT_ROOT / "uv.lock").read_text().splitlines()
    package_index = next(i for i, line in enumerate(lock_lines) if line == 'name = "capsem"')
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


def test_release_commands_require_source_commit_without_a_parallel_just_surface() -> None:
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
    assert '\nrelease-binaries channel source_commit force="false":' in justfile
    assert '\nrelease-profile channel profile source_commit force="false":' in justfile


def test_binary_release_recipe_uses_one_adversarial_script() -> None:
    justfile = _read_text_exact_case("justfile")
    script = _read_text_exact_case("scripts/release-binaries.py")

    # One adversarial script owns the publish, and the release plan reaches it
    # without rebuilding assets on the way -- read from the plan, because the
    # recipe is now a dispatch and the sequence lives in the graph.
    binary_plan = _planned("release-binaries", channel="nightly")
    assert "scripts/release-binaries.py" in binary_plan
    assert "_build-kernel" not in binary_plan
    assert "_build-rootfs" not in binary_plan
    assert '\nrelease-binaries channel source_commit force="false":' in justfile
    assert "MUTATED_PATHS" not in script
    assert '"push", "origin", "main"' not in script
    assert '"reset"' not in script
    assert '"commit"' not in script
    assert "SOURCE_REF_TEMPLATE.format(source_commit=source_commit)" in script
    assert '"workflow",\n            "run",\n            "release.yaml"' in script
    assert 'f"source_commit={source_commit}"' in script
    assert "release-assets.yaml" not in script

"""The workspace version is read from its authority, never reconstructed.

Two shapes cost a release each. `grep '^version' Cargo.toml | head -1` matches
the first line in the file that starts with `version`, wherever it lives, so it
is a bet on table order rather than a parse. And a scheme that appended
`$(date +%s)` produced versions that sorted but meant nothing, and that sat
above every compatibility floor by accident.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
from capsem_builder.gate import versions
from capsem_builder.gate.errors import GateError
from helpers.gate import RecordingRunner

WORKSPACE = """\
[workspace]
members = ["crates/capsem"]

[workspace.package]
version = "{version}"
edition = "2021"
"""

UV_LOCK = """\
version = 1
revision = 3
requires-python = ">=3.11"

[[package]]
name = "capsem"
version = "0.0.1"
source = { editable = "." }

[[package]]
name = "dependency"
version = "7.8.9"
source = { registry = "https://pypi.org/simple" }
"""


PROJECT_ROOT = Path(__file__).resolve().parents[3]


def _checkout(tmp_path: Path, *, version: str = "9.9.9", cargo: str | None = None) -> Path:
    (tmp_path / "config").mkdir(parents=True, exist_ok=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    (tmp_path / "Cargo.toml").write_text(cargo or WORKSPACE.format(version=version))
    (tmp_path / "pyproject.toml").write_text(
        '[project]\nname = "capsem"\nversion = "0.0.1"\n'
    )
    (tmp_path / "crates" / "capsem-app").mkdir(parents=True)
    (tmp_path / "crates" / "capsem-app" / "tauri.conf.json").write_text(
        '{\n  "productName": "Capsem",\n  "version": "0.0.1"\n}\n'
    )
    (tmp_path / "uv.lock").write_text(UV_LOCK)
    return tmp_path


# ---------------------------------------------------------------------------
# Reading
# ---------------------------------------------------------------------------


def test_reads_the_workspace_package_version(tmp_path: Path) -> None:
    root = _checkout(tmp_path, version="4.2.0")

    assert versions.workspace_version(root) == "4.2.0"


def test_an_earlier_version_key_does_not_win(tmp_path: Path) -> None:
    """The exact failure `grep '^version' | head -1` was one edit away from.

    Any table added above `[workspace.package]` that declares a version -- a
    patch entry, a workspace dependency pin -- would have been picked up as the
    release version, and the release would have named itself after a
    dependency.
    """
    root = _checkout(
        tmp_path,
        cargo=(
            '[workspace.dependencies.serde]\n'
            'version = "1.0.200"\n'
            "\n"
            "[workspace.package]\n"
            'version = "4.2.0"\n'
        ),
    )

    assert versions.workspace_version(root) == "4.2.0"


def test_a_checkout_without_a_workspace_version_says_so(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    (root / "Cargo.toml").write_text('[package]\nname = "capsem"\n')

    with pytest.raises(GateError, match=r"no .workspace.package. version"):
        versions.workspace_version(root)


@pytest.mark.parametrize("bad", ["1.6", "v1.6.0", "1.6.0-rc1", ""])
def test_non_semver_is_refused(bad: str) -> None:
    with pytest.raises(GateError, match="not semver"):
        versions.require_semver(bad, source="Cargo.toml")


@pytest.mark.parametrize("dated", ["2026.0730.16", "1.06.0", "0730.1.0"])
def test_a_date_shaped_version_is_refused_for_its_leading_zero(dated: str) -> None:
    """`2026.0730.16` was a real asset version, and `\\d+\\.\\d+\\.\\d+` accepts it.

    Semver forbids leading zeros in numeric identifiers, which is exactly the
    rule that separates a zero-padded date from a decision someone made. A
    version this shape sorts above every compatibility floor it should have
    been compared against.
    """
    with pytest.raises(GateError, match="not semver"):
        versions.require_semver(dated, source="the asset manifest")


# ---------------------------------------------------------------------------
# Stamping
# ---------------------------------------------------------------------------


def test_stamp_fans_the_one_version_out_to_the_cohort(tmp_path: Path) -> None:
    root = _checkout(tmp_path, version="4.2.0")
    runner = RecordingRunner(root, failures=["rev-parse"])

    assert versions.stamp(root, runner) == "4.2.0"

    assert '"version": "4.2.0"' in (
        root / "crates" / "capsem-app" / "tauri.conf.json"
    ).read_text()
    assert 'version = "4.2.0"' in (root / "pyproject.toml").read_text()
    uv_lock = (root / "uv.lock").read_text()
    assert 'name = "capsem"\nversion = "4.2.0"' in uv_lock
    assert 'name = "dependency"\nversion = "7.8.9"' in uv_lock
    assert runner.ran(r"uv lock --locked --offline")


def test_stamp_refreshes_both_lockfiles_after_substituting(tmp_path: Path) -> None:
    """Order matters: a lock refreshed before the edit locks the old version."""
    root = _checkout(tmp_path)
    runner = RecordingRunner(root, failures=["rev-parse"])

    versions.stamp(root, runner)

    runner.assert_order(r"cargo update --workspace --offline", r"uv lock --locked --offline")


def test_stamp_refuses_a_uv_lock_without_one_editable_capsem_root(tmp_path: Path) -> None:
    root = _checkout(tmp_path)
    (root / "uv.lock").write_text(UV_LOCK.replace('name = "capsem"', 'name = "other"'))
    runner = RecordingRunner(root, failures=["rev-parse"])

    with pytest.raises(GateError, match="one editable capsem root"):
        versions.stamp(root, runner)

    assert not runner.ran(r"uv lock")


def test_stamp_refuses_a_version_that_is_already_tagged(tmp_path: Path) -> None:
    root = _checkout(tmp_path, version="4.2.0")
    # `git rev-parse --verify refs/tags/v4.2.0` succeeding means the tag exists.
    runner = RecordingRunner(root)

    with pytest.raises(GateError, match="is already tagged"):
        versions.stamp(root, runner)

    assert not runner.ran(r"cargo update"), (
        "a refused stamp must not have edited anything first"
    )


def test_stamp_fails_when_a_cohort_file_stopped_spelling_the_version(
    tmp_path: Path,
) -> None:
    """`sed` was happy to match nothing and report success.

    A renamed key would have left that file on the previous release's version
    while every other file moved -- the exact split-cohort state that made
    `min_binary` disagree with the binary it shipped beside.
    """
    root = _checkout(tmp_path)
    (root / "crates" / "capsem-app" / "tauri.conf.json").write_text(
        '{\n  "appVersion": "0.0.1"\n}\n'
    )
    runner = RecordingRunner(root, failures=["rev-parse"])

    with pytest.raises(GateError, match="exactly once, matched 0 times"):
        versions.stamp(root, runner)


def test_stamping_is_a_pure_function_of_the_declared_version(tmp_path: Path) -> None:
    """Stamp twice, get identical bytes: nothing here reads a clock.

    The retired scheme appended `$(date +%s)`, so two stamps of one unchanged
    checkout produced two different releases. `build_system/tests/release/test_retired_version_formats.py`
    scans this package for that shape; this asserts the behaviour it implies.
    """
    root = _checkout(tmp_path, version="4.2.0")
    runner = RecordingRunner(root, failures=["rev-parse"])

    versions.stamp(root, runner)
    first = (root / "pyproject.toml").read_text()
    versions.stamp(root, runner)

    assert (root / "pyproject.toml").read_text() == first


def test_the_stamped_version_carries_no_clock_component(tmp_path: Path) -> None:
    root = _checkout(tmp_path, version="4.2.0")
    runner = RecordingRunner(root, failures=["rev-parse"])

    stamped = versions.stamp(root, runner)

    assert re.fullmatch(r"\d+\.\d+\.\d+", stamped)
    assert int(stamped.split(".")[2]) < 1000, (
        "a patch component in the billions is a Unix timestamp, not a decision"
    )

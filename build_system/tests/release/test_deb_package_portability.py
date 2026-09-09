"""Rehearsal reads package identities without native Debian programs."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from capsem_builder.release.tools import (
    local_release_glowup,
    release_installed_probe,
    stage_release_test_inputs,
)
from capsem_builder.release.tools.finalize_binary_staging_fixtures import _write_synthetic_deb


@pytest.fixture
def package(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    tree = tmp_path / "tree"
    control = tree / "DEBIAN/control"
    control.parent.mkdir(parents=True)
    control.write_text("Package: capsem\nVersion: 1.2.3\nArchitecture: arm64\n")
    metadata = tree / "usr/share/capsem/assets/manifest-metadata.json"
    metadata.parent.mkdir(parents=True)
    metadata.write_text(json.dumps({"manifest_url": "https://example.test/manifest.json", "channel": "stable"}))
    binary = tree / "usr/bin/capsem"
    binary.parent.mkdir(parents=True)
    binary.write_bytes(b"exact package binary")
    output = tmp_path / "candidate.deb"
    _write_synthetic_deb(tree, output)
    monkeypatch.setenv("PATH", str(tmp_path / "no-host-tools"))
    return output


@pytest.mark.parametrize("field,expected", [("version", "1.2.3"), ("arch", "arm64")])
def test_rehearsal_reads_exact_deb_fields_without_host_tools(package: Path, field: str, expected: str) -> None:
    assert getattr(local_release_glowup, f"deb_{field}")(package) == expected


def test_installed_probe_reads_package_metadata_without_extracting_on_host(package: Path) -> None:
    assert release_installed_probe.packaged_manifest_metadata(package) == {
        "manifest_url": "https://example.test/manifest.json", "channel": "stable",
    }


def test_candidate_binary_staging_needs_no_host_debian_tools(package: Path) -> None:
    output = package.parent / "staged"
    output.mkdir()
    (output / "capsem-source-only").write_bytes(b"stale fallback")
    staged = stage_release_test_inputs.stage_candidate_package(package, output)
    assert staged == [output / "capsem"]
    assert staged[0].read_bytes() == b"exact package binary"
    assert staged[0].stat().st_mode & 0o777 == 0o755
    assert not (output / "capsem-source-only").exists()


@pytest.mark.parametrize("version_fields", ["", "Version: 1.2.3\nVersion: 4.5.6\n"])
def test_deb_version_refuses_missing_or_ambiguous_identity(package: Path, version_fields: str) -> None:
    tree = package.parent / "tree"
    (tree / "DEBIAN/control").write_text(f"Package: capsem\n{version_fields}Architecture: arm64\n")
    package.unlink()
    _write_synthetic_deb(tree, package)
    with pytest.raises(SystemExit, match="exactly one nonempty Version"):
        local_release_glowup.deb_version(package)


@pytest.mark.parametrize("count", [0, 2])
def test_manifest_metadata_requires_one_package_owned_member(package: Path, count: int) -> None:
    tree = package.parent / "tree"
    metadata = tree / "usr/share/capsem/assets/manifest-metadata.json"
    if count == 0:
        metadata.unlink()
    else:
        second = tree / "usr/local/share/capsem/assets/manifest-metadata.json"
        second.parent.mkdir(parents=True)
        second.write_bytes(metadata.read_bytes())
    package.unlink()
    _write_synthetic_deb(tree, package)
    with pytest.raises(SystemExit, match=f"exactly one manifest metadata, found {count}"):
        release_installed_probe.packaged_manifest_metadata(package)

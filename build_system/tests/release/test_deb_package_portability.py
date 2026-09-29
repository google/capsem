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


def test_installed_probe_checks_the_binaries_the_package_ships(package: Path) -> None:
    """A transition installs an older package whose binary set differs.

    0.6.3 shipped capsem-port-router (and no capsem-router); the probe checked
    every current binary name against it and failed the release glow-up on a
    binary that package never had. It checks what the package ships.
    """
    tree = package.parent / "tree"
    for name in ("capsem-service", "capsem-port-router", "capsem-app"):
        (tree / "usr/bin" / name).write_bytes(b"old package binary")
    package.unlink()
    _write_synthetic_deb(tree, package)

    assert release_installed_probe.packaged_host_binaries(package) == ("capsem", "capsem-service")


def test_installed_probe_shell_checks_each_artifact_payload(tmp_path: Path) -> None:
    shell = release_installed_probe.exact_installed_probe_shell(tmp_path)
    assert 'check_binary_versions "$package_version" "$artifact"' in shell
    assert "packaged_host_binaries" in shell


def _public_transition_version_check(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> str:
    """The rendered public transition script's binary check, as bash."""
    import re

    from capsem_builder.release.tools import check_public_binary_release as public

    scripts: list[str] = []
    monkeypatch.setattr(public.shutil, "which", lambda _name: "/usr/bin/docker")
    monkeypatch.setattr(
        public, "linux_amd64_current_package", lambda manifest: {"version": manifest["version"]}
    )
    monkeypatch.setattr(public.subprocess, "run", lambda argv, **_: scripts.append(argv[-1]))
    public.run_docker_binary_transition_smoke(
        older_manifest={"version": "0.6.3"},
        newer_manifest={"version": "0.6.4"},
        install_script_url="http://127.0.0.1:1/install.sh",
        docker_image="unused",
        work_dir=tmp_path / "work",
    )
    monkeypatch.undo()  # the stubs patched the shared subprocess module
    match = re.search(r"^check_binary_versions\(\) \{\n.*?^\}\n", scripts[0], re.S | re.M)
    assert match, scripts[0]
    return match.group(0)


@pytest.mark.parametrize("owned_router", [False, True])
def test_public_transition_checks_the_binaries_the_installed_package_owns(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, owned_router: bool
) -> None:
    """The older public package predates capsem-router; checking the current
    binary names against it failed a release on a binary it never shipped.
    A binary the installed package does own must still be present."""
    import subprocess

    function = _public_transition_version_check(tmp_path, monkeypatch)
    home = tmp_path / "home"
    (home / ".capsem/bin").mkdir(parents=True)
    for name in ("capsem", "capsem-service"):
        tool = home / ".capsem/bin" / name
        tool.write_text("#!/bin/sh\necho " + name + " 0.6.3\n")
        tool.chmod(0o755)
    owned = ["/usr/bin/capsem", "/usr/bin/capsem-service"] + (["/usr/bin/capsem-router"] if owned_router else [])
    stubs = tmp_path / "stubs"
    stubs.mkdir()
    (stubs / "dpkg").write_text("#!/bin/sh\nprintf '%s\\n' " + " ".join(owned) + "\n")
    (stubs / "su").write_text('#!/bin/sh\nshift; exec sh -c "$2"\n')
    for stub in stubs.iterdir():
        stub.chmod(0o755)
    result = subprocess.run(
        ["bash", "-c", "set -euo pipefail\n" + function + "\ncheck_binary_versions 0.6.3\n"],
        env={"PATH": f"{stubs}:/usr/bin:/bin", "HOME": str(home)},
        capture_output=True,
        text=True,
        check=False,
    )
    assert (result.returncode == 0) is (not owned_router), result.stderr

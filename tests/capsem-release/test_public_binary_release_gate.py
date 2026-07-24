"""Post-deploy binary release gate contract tests."""

from __future__ import annotations

import gzip
import hashlib
import io
import importlib.util
import json
import re
import subprocess
import sys
import tarfile
from types import ModuleType
from pathlib import Path

import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "check-public-binary-release.py"


def _workflow_job_blocks(workflow: str) -> dict[str, str]:
    blocks: dict[str, str] = {}
    matches = list(re.finditer(r"(?m)^  ([a-zA-Z0-9_-]+):\n", workflow))
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(workflow)
        blocks[match.group(1)] = workflow[match.start() : end]
    return blocks


def _load_release_gate() -> ModuleType:
    spec = importlib.util.spec_from_file_location("check_public_binary_release", SCRIPT)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_public_binary_release_gate_fetch_retries_ipv4_on_network_unreachable(
    monkeypatch,
) -> None:
    gate = _load_release_gate()
    calls: list[str] = []

    class FakeResponse:
        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def read(self) -> bytes:
            return b"ok"

    def fake_urlopen(request, *, timeout: int):
        calls.append(request.full_url)
        assert timeout == 120
        if len(calls) == 1:
            raise gate.urllib.error.URLError(
                OSError(gate.errno.ENETUNREACH, "Network is unreachable")
            )
        return FakeResponse()

    monkeypatch.setattr(gate.urllib.request, "urlopen", fake_urlopen)

    assert gate.fetch_bytes("https://release.capsem.org/assets/stable/manifest.json") == b"ok"
    assert calls == [
        "https://release.capsem.org/assets/stable/manifest.json",
        "https://release.capsem.org/assets/stable/manifest.json",
    ]


def test_public_binary_release_gate_rejects_machine_architecture_as_package_identity() -> None:
    gate = _load_release_gate()

    with pytest.raises(gate.argparse.ArgumentTypeError, match="package architecture"):
        gate.RequiredPackage.parse("linux:x86_64:debian_package")

    required = gate.RequiredPackage.parse("linux:amd64:debian_package")
    assert required.architecture is gate.PackageArchitecture.AMD64


def test_public_binary_release_gate_reads_package_contents(tmp_path: Path) -> None:
    package_dir = tmp_path / "packages"
    package_dir.mkdir()
    package = package_dir / "Capsem_9.9.9_amd64.deb"
    capsem = b"#!/bin/sh\necho capsem 9.9.9\n"
    admin = b"#!/bin/sh\necho capsem-admin 9.9.9\n"
    manifest_url = (tmp_path / "manifest.json").resolve().as_uri()
    _write_minimal_deb(
        package,
        {"usr/bin/capsem": capsem, "usr/bin/capsem-admin": admin},
        manifest_url=manifest_url,
    )

    _write_manifest(
        tmp_path,
        [
            _package_record(
                "amd64",
                package.name,
                package,
                [
                    _binary_record("capsem", "/usr/bin/capsem", capsem),
                    _binary_record("capsem-admin", "/usr/bin/capsem-admin", admin),
                ],
            )
        ],
    )
    install_sh = _write_install_sh(tmp_path)

    result = subprocess.run(
        [
            "uv",
            "run",
            "python",
            str(SCRIPT),
            "--manifest-url",
            manifest_url,
            "--install-script-url",
            str(install_sh),
            "--package-dir",
            str(package_dir),
            "--required-package",
            "linux:amd64:debian_package",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert result.returncode == 0, result.stderr
    assert "validated 1 package and 2 packaged binaries" in result.stdout


def test_public_binary_release_gate_rejects_manifest_binary_hash_drift(
    tmp_path: Path,
) -> None:
    package_dir = tmp_path / "packages"
    package_dir.mkdir()
    package = package_dir / "Capsem_9.9.9_amd64.deb"
    capsem = b"#!/bin/sh\necho capsem 9.9.9\n"
    manifest_url = (tmp_path / "manifest.json").resolve().as_uri()
    _write_minimal_deb(package, {"usr/bin/capsem": capsem}, manifest_url=manifest_url)

    wrong_contents = b"#!/bin/sh\necho not-the-packaged-file\n"
    _write_manifest(
        tmp_path,
        [
            _package_record(
                "amd64",
                package.name,
                package,
                [_binary_record("capsem", "/usr/bin/capsem", wrong_contents)],
            )
        ],
    )
    install_sh = _write_install_sh(tmp_path)

    result = subprocess.run(
        [
            "uv",
            "run",
            "python",
            str(SCRIPT),
            "--manifest-url",
            manifest_url,
            "--install-script-url",
            str(install_sh),
            "--package-dir",
            str(package_dir),
            "--required-package",
            "linux:amd64:debian_package",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert result.returncode != 0
    assert "binary /usr/bin/capsem SHA-256 not found inside Capsem_9.9.9_amd64.deb" in (
        result.stderr + result.stdout
    )


def test_public_binary_release_gate_rejects_packaged_binary_version_drift(
    tmp_path: Path,
) -> None:
    gate = _load_release_gate()
    capsem = b"#!/bin/sh\necho capsem 9.9.8\n"
    failures = gate.check_packaged_binary_version(
        {"version": "9.9.9"},
        _binary_record("capsem", "/usr/bin/capsem", capsem),
        "/usr/bin/capsem",
        {"/usr/bin/capsem": capsem},
        tmp_path,
        "Capsem_9.9.9_amd64.deb",
    )

    assert failures == [
        "binary /usr/bin/capsem version output does not contain 9.9.9: capsem 9.9.8"
    ]


def test_public_binary_release_gate_does_not_execute_gui_payload_without_deps(
    tmp_path: Path,
) -> None:
    package_dir = tmp_path / "packages"
    package_dir.mkdir()
    package = package_dir / "Capsem_9.9.9_amd64.deb"
    capsem = b"#!/bin/sh\necho capsem 9.9.9\n"
    app = b"#!/bin/sh\necho missing libwebkit2gtk >&2\nexit 127\n"
    tray = b"#!/bin/sh\necho missing libxdo >&2\nexit 127\n"
    manifest_url = (tmp_path / "manifest.json").resolve().as_uri()
    _write_minimal_deb(
        package,
        {
            "usr/bin/capsem": capsem,
            "usr/bin/capsem-app": app,
            "usr/bin/capsem-tray": tray,
        },
        manifest_url=manifest_url,
    )

    manifest = _write_manifest(
        tmp_path,
        [
            _package_record(
                "amd64",
                package.name,
                package,
                [
                    _binary_record("capsem", "/usr/bin/capsem", capsem),
                    _binary_record("capsem-app", "/usr/bin/capsem-app", app),
                    _binary_record("capsem-tray", "/usr/bin/capsem-tray", tray),
                ],
            )
        ],
    )
    install_sh = _write_install_sh(tmp_path)

    result = subprocess.run(
        [
            "uv",
            "run",
            "python",
            str(SCRIPT),
            "--manifest-url",
            manifest.resolve().as_uri(),
            "--install-script-url",
            str(install_sh),
            "--package-dir",
            str(package_dir),
            "--required-package",
            "linux:amd64:debian_package",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert result.returncode == 0, result.stderr
    assert "validated 1 package and 3 packaged binaries" in result.stdout


def test_public_binary_release_gate_rejects_frozen_manifest_payload(tmp_path: Path) -> None:
    package_dir = tmp_path / "packages"
    package_dir.mkdir()
    package = package_dir / "Capsem_9.9.9_amd64.deb"
    capsem = b"#!/bin/sh\necho capsem 9.9.9\n"
    manifest_url = (tmp_path / "manifest.json").resolve().as_uri()
    _write_minimal_deb(
        package,
        {
            "usr/bin/capsem": capsem,
            "usr/share/capsem/assets/manifest.json": b'{"format":2}\n',
        },
        manifest_url=manifest_url,
    )
    manifest = _write_manifest(
        tmp_path,
        [
            _package_record(
                "amd64",
                package.name,
                package,
                [_binary_record("capsem", "/usr/bin/capsem", capsem)],
            )
        ],
    )
    install_sh = _write_install_sh(tmp_path)

    result = subprocess.run(
        [
            "uv",
            "run",
            "python",
            str(SCRIPT),
            "--manifest-url",
            manifest.resolve().as_uri(),
            "--install-script-url",
            str(install_sh),
            "--package-dir",
            str(package_dir),
            "--required-package",
            "linux:amd64:debian_package",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert result.returncode != 0
    assert "freezes /usr/share/capsem/assets/manifest.json" in (result.stderr + result.stdout)


def test_public_binary_release_gate_rejects_manifest_metadata_package_version_drift(
    tmp_path: Path,
) -> None:
    package_dir = tmp_path / "packages"
    package_dir.mkdir()
    package = package_dir / "Capsem_9.9.9_amd64.deb"
    capsem = b"#!/bin/sh\necho capsem 9.9.9\n"
    manifest_url = (tmp_path / "manifest.json").resolve().as_uri()
    _write_minimal_deb(
        package,
        {"usr/bin/capsem": capsem},
        manifest_url=manifest_url,
        package_version="9.9.8",
    )
    manifest = _write_manifest(
        tmp_path,
        [
            _package_record(
                "amd64",
                package.name,
                package,
                [_binary_record("capsem", "/usr/bin/capsem", capsem)],
            )
        ],
    )
    install_sh = _write_install_sh(tmp_path)

    result = subprocess.run(
        [
            "uv",
            "run",
            "python",
            str(SCRIPT),
            "--manifest-url",
            manifest.resolve().as_uri(),
            "--install-script-url",
            str(install_sh),
            "--package-dir",
            str(package_dir),
            "--required-package",
            "linux:amd64:debian_package",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert result.returncode != 0
    assert "manifest-metadata package_version '9.9.8' does not match 9.9.9" in (
        result.stderr + result.stdout
    )


def test_release_workflow_runs_public_package_gate_and_native_install() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8")
    verify_downloads = workflow.split("  verify-release-downloads:", maxsplit=1)[1]

    assert "scripts/check-public-binary-release.py" in verify_downloads
    assert '--channel "$RELEASE_CHANNEL"' in verify_downloads
    assert '--manifest-url "$ASSET_MANIFEST_URL"' in verify_downloads
    assert (
        "--stable-manifest-url https://release.capsem.org/assets/stable/manifest.json"
    ) in verify_downloads
    assert (
        "--nightly-manifest-url https://release.capsem.org/assets/nightly/manifest.json"
    ) in verify_downloads
    assert "--install-script-url https://capsem.org/install.sh" in verify_downloads
    assert "--site-url https://capsem.org/" in verify_downloads
    assert "--docker-linux-install" not in verify_downloads
    assert "Enable KVM for live public-install VM proof" in verify_downloads
    assert (
        'curl -fsSL https://capsem.org/install.sh | CAPSEM_CHANNEL="$RELEASE_CHANNEL" sh'
        in verify_downloads
    )
    assert "scripts/prove-installed-shell.py" in verify_downloads
    assert "CAPSEM_LIVE_PUBLIC_INSTALL_SHELL_OK" in verify_downloads
    assert "scripts/verify-installed-release.py" in verify_downloads
    assert '"$HOME/.capsem/bin/capsem" run' not in verify_downloads


def test_release_workflow_verifies_exact_installed_state_before_artifact_publication() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8")
    jobs = _workflow_job_blocks(workflow)

    for job_name in ("build-app-macos", "build-app-linux"):
        job = jobs[job_name]
        assert "scripts/verify-installed-release.py" in job
        assert job.index("scripts/verify-installed-release.py") < job.index(
            "Collect macOS artifacts" if job_name == "build-app-macos" else "Collect Linux artifacts"
        )

    assert jobs["create-release"].index("needs: [build-app-macos, build-app-linux]") >= 0


def test_public_binary_release_gate_keeps_public_installer_default_on_stable() -> None:
    gate = _load_release_gate()

    failures = gate.check_install_script_defaults(
        "\n".join(
            (
                'CAPSEM_CHANNEL="${CAPSEM_CHANNEL:-stable}"',
                'CAPSEM_RELEASE_BASE_URL="${CAPSEM_RELEASE_BASE_URL:-https://release.capsem.org}"',
                "/assets/${CAPSEM_CHANNEL}/manifest.json",
                "ASSET_BYTES ASSET_SHA256 verify_package",
                'sudo /usr/sbin/installer -pkg "$PKG_PATH" -target /',
                'sudo apt install -y "$DEB_PATH"',
            )
        ),
        release_base_url="https://release.capsem.org",
    )

    assert failures == []


def test_public_binary_release_gate_switches_stable_to_nightly_and_back(
    monkeypatch,
) -> None:
    gate = _load_release_gate()
    calls: list[list[str]] = []

    monkeypatch.setattr(gate.shutil, "which", lambda _name: "/usr/bin/docker")

    def capture_run(args, **_kwargs):
        calls.append(args)
        return subprocess.CompletedProcess(args, 0)

    monkeypatch.setattr(gate.subprocess, "run", capture_run)
    stable = "https://release.capsem.org/assets/stable/manifest.json"
    nightly = "https://release.capsem.org/assets/nightly/manifest.json"

    gate.run_docker_install_smoke(
        release_base_url="https://release.capsem.org",
        install_script_url="https://capsem.org/install.sh",
        stable_manifest_url=stable,
        nightly_manifest_url=nightly,
        expected_version="1.5.0",
        channel_switch=True,
        upgrade=True,
        docker_image="ubuntu:24.04",
    )

    script = calls[0][7]
    assert "CAPSEM_CHANNEL=stable" in script
    assert "update --assets --channel stable" in script
    initial_stable = script.index(f"grep -F {stable}")
    switch_nightly = script.index("update --assets --channel nightly")
    verify_nightly = script.index(f"grep -F {nightly}", switch_nightly)
    switch_stable = script.index("update --assets --channel stable", verify_nightly)
    verify_stable = script.index(f"grep -F {stable}", switch_stable)
    upgrade_nightly = script.index("update --yes --channel nightly")
    assert initial_stable < switch_nightly < verify_nightly < switch_stable
    assert switch_stable < verify_stable < upgrade_nightly
    assert "dpkg-query -W -f='${Version}' capsem | grep -Fx 1.5.0" in script
    assert 'grep -F "Installed: true" /home/capsemtest/.capsem/service-status.txt' in script
    assert 'grep -F "Running:   true" /home/capsemtest/.capsem/service-status.txt' in script
    assert 'grep -F "Service:   ok" /home/capsemtest/.capsem/service-status.txt' in script
    assert 'grep -F "Gateway:   ok" /home/capsemtest/.capsem/service-status.txt' in script


def test_public_binary_release_gate_runs_install_switch_and_upgrade_paths() -> None:
    source = SCRIPT.read_text(encoding="utf-8")

    assert "--docker-channel-switch" in source
    assert "--docker-upgrade" in source
    assert "--docker-transition-from-manifest" in source
    assert "update --assets --channel nightly" in source
    assert "update --assets --channel stable" in source
    assert "CAPSEM_RELEASE_CHANNELS_URL=" in source
    assert "update --yes --channel nightly" in source
    assert '.capsem/bin/$bin\\\\" --version' in source
    assert "manifest-metadata.json" in source
    assert "snapshot_sha256" in source
    assert "freezes {frozen_manifest_path}" in source
    assert "should_execute_packaged_binary" in source
    assert "check_packaged_binary_version" in source
    assert "--site-url" in source
    assert "check_public_site_download_links" in source

    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    assert "name: binary-channel-before" in workflow
    assert "target/binary-channel/*/manifest.before.json" in workflow
    assert '--docker-transition-from-manifest "/tmp/binary-channel-before/$RELEASE_CHANNEL/manifest.before.json"' in workflow


def test_public_binary_transition_gate_uses_two_real_manifests_and_downgrades(
    monkeypatch,
    tmp_path: Path,
) -> None:
    gate = _load_release_gate()
    calls: list[list[str]] = []
    monkeypatch.setattr(gate.shutil, "which", lambda _name: "/usr/bin/docker")
    monkeypatch.setattr(
        gate.subprocess,
        "run",
        lambda args, **_kwargs: calls.append(args) or subprocess.CompletedProcess(args, 0),
    )
    def package(version: str) -> dict[str, object]:
        return {
            "name": f"Capsem_{version}_amd64.deb",
            "version": version,
            "kind": "debian_package",
            "platform": "linux",
            "architecture": "amd64",
            "status": "current",
            "url": f"https://example.test/v{version}/Capsem_{version}_amd64.deb",
            "bytes": 100,
            "digest": {"sha256": "1" * 64, "blake3": "2" * 64},
        }
    older = {"version": "1.0.1", "packages": [package("1.5.100")], "profiles": {}}
    newer = {"version": "1.0.2", "packages": [package("1.5.101")], "profiles": {}}

    gate.run_docker_binary_transition_smoke(
        older_manifest=older,
        newer_manifest=newer,
        install_script_url="https://capsem.org/install.sh",
        docker_image="ubuntu:24.04",
        work_dir=tmp_path,
    )

    script = calls[0][-1]
    assert "CAPSEM_CHANNEL=stable" in script
    assert "update --yes --channel nightly" in script
    assert "check_installed_version 1.5.101" in script
    assert "update --yes --channel stable" in script
    assert script.count("check_installed_version 1.5.100") == 2
    assert "dpkg-query -W -f='${Version}' capsem | grep -Fx \"$expected\"" in script
    assert 'check_binary_versions "$expected"' in script
    assert script.count("scripts/verify-installed-release.py") == 3


def test_public_binary_release_gate_requires_fail_closed_installer_integrity() -> None:
    gate = _load_release_gate()
    script = (PROJECT_ROOT / "site" / "public" / "install.sh").read_text(encoding="utf-8")

    failures = gate.check_install_script_defaults(
        script,
        release_base_url="https://release.capsem.org",
    )

    assert failures == []
    for required in (
        "ASSET_BYTES",
        "ASSET_SHA256",
        "verify_package",
        "sudo /usr/sbin/installer -pkg",
        "sudo apt install -y",
    ):
        assert required in script


def test_public_binary_release_gate_accepts_stable_site_download_entrypoint() -> None:
    gate = _load_release_gate()

    failures = gate.check_public_site_download_links(
        """
        <a href="https://capsem.org/install.sh">Install</a>
        <code>https://release.capsem.org/assets/stable/manifest.json</code>
        """,
        site_url="file:///site.html",
        channel="stable",
        release_base_url="https://release.capsem.org",
    )

    assert failures == []


def test_public_binary_release_gate_rejects_asset_tag_site_download_url() -> None:
    gate = _load_release_gate()

    failures = gate.check_public_site_download_links(
        """
        <a href="https://github.com/google/capsem/releases/tag/assets-v2026.0703.2">
          Download DMG
        </a>
        """,
        site_url="file:///site.html",
        channel="stable",
        release_base_url="https://release.capsem.org",
    )

    assert any("asset-release tag" in failure for failure in failures)


def _write_manifest(tmp_path: Path, packages: list[dict[str, object]]) -> Path:
    path = tmp_path / "manifest.json"
    path.write_text(
        json.dumps({"schema": "capsem.release_graph.v1", "packages": packages}, indent=2),
        encoding="utf-8",
    )
    return path


def _write_install_sh(tmp_path: Path) -> Path:
    path = tmp_path / "install.sh"
    path.write_text(
        "\n".join(
            [
                'CAPSEM_CHANNEL="${CAPSEM_CHANNEL:-stable}"',
                'CAPSEM_RELEASE_BASE_URL="${CAPSEM_RELEASE_BASE_URL:-https://release.capsem.org}"',
                'CAPSEM_MANIFEST_URL="${CAPSEM_MANIFEST_URL:-${CAPSEM_RELEASE_BASE_URL}/assets/${CAPSEM_CHANNEL}/manifest.json}"',
                "ASSET_BYTES=1",
                "ASSET_SHA256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "verify_package() { :; }",
                'sudo /usr/sbin/installer -pkg "$PKG_PATH" -target /',
                'sudo apt install -y "$DEB_PATH"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    return path


def _package_record(
    arch: str,
    name: str,
    path: Path,
    binaries: list[dict[str, object]],
) -> dict[str, object]:
    payload = path.read_bytes()
    return {
        "architecture": arch,
        "binaries": binaries,
        "bytes": len(payload),
        "digest": {"sha256": hashlib.sha256(payload).hexdigest()},
        "kind": "debian_package",
        "name": name,
        "platform": "linux",
        "status": "current",
        "url": f"https://github.com/google/capsem/releases/download/v9.9.9/{name}",
        "version": "9.9.9",
    }


def _binary_record(name: str, installed_path: str, contents: bytes) -> dict[str, object]:
    return {
        "bytes": len(contents),
        "digest": {"sha256": hashlib.sha256(contents).hexdigest()},
        "installed_path": installed_path,
        "name": name,
        "sbom_component_ref": f"SPDXRef-File-{name}",
        "version": "9.9.9",
    }


def _write_minimal_deb(
    path: Path,
    members: dict[str, bytes],
    *,
    manifest_url: str,
    package_version: str = "9.9.9",
) -> None:
    origin = json.dumps(
        {
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": manifest_url,
            "package_version": package_version,
        },
        sort_keys=True,
    ).encode()
    members = {
        **members,
        "usr/share/capsem/assets/manifest-metadata.json": origin,
    }
    data_tar = io.BytesIO()
    with gzip.GzipFile(fileobj=data_tar, mode="wb", mtime=0) as gz:
        with tarfile.open(fileobj=gz, mode="w") as tar:
            for member_path, contents in members.items():
                info = tarfile.TarInfo(member_path)
                info.mode = 0o755
                info.size = len(contents)
                info.mtime = 0
                tar.addfile(info, io.BytesIO(contents))
    control_tar = io.BytesIO()
    with gzip.GzipFile(fileobj=control_tar, mode="wb", mtime=0) as gz:
        with tarfile.open(fileobj=gz, mode="w") as tar:
            control = (
                f"Package: capsem\nVersion: {package_version}\nArchitecture: amd64\n"
            ).encode()
            info = tarfile.TarInfo("control")
            info.mode = 0o644
            info.size = len(control)
            info.mtime = 0
            tar.addfile(info, io.BytesIO(control))
    deb = (
        b"!<arch>\n"
        + _ar_member("debian-binary", b"2.0\n")
        + _ar_member("control.tar.gz", control_tar.getvalue())
        + _ar_member("data.tar.gz", data_tar.getvalue())
    )
    path.write_bytes(deb)


def _ar_member(name: str, data: bytes) -> bytes:
    header = (f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{100644:<8}{len(data):<10}`\n").encode("ascii")
    return header + data + (b"\n" if len(data) % 2 else b"")

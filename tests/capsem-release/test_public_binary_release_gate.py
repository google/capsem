"""Post-deploy binary release gate contract tests."""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import subprocess
import tarfile
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "check-public-binary-release.py"


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
                "x86_64",
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
            "linux:x86_64:debian_package",
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
                "x86_64",
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
            "linux:x86_64:debian_package",
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
                "x86_64",
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
            "linux:x86_64:debian_package",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert result.returncode != 0
    assert "freezes /usr/share/capsem/assets/manifest.json" in (result.stderr + result.stdout)


def test_release_workflow_runs_public_package_gate_and_docker_install() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8")
    verify_downloads = workflow.split("  verify-release-downloads:", maxsplit=1)[1]

    assert "scripts/check-public-binary-release.py" in verify_downloads
    assert "--channel stable" in verify_downloads
    assert "--manifest-url \"$ASSET_MANIFEST_URL\"" in verify_downloads
    assert "--install-script-url https://capsem.org/install.sh" in verify_downloads
    assert "--docker-linux-install" in verify_downloads
    assert "--docker-channel-switch" in verify_downloads
    assert "--docker-upgrade" in verify_downloads
    assert "curl -fsSL https://capsem.org/install.sh | sh" in verify_downloads


def test_public_binary_release_gate_runs_install_switch_and_upgrade_paths() -> None:
    source = SCRIPT.read_text(encoding="utf-8")

    assert "--docker-channel-switch" in source
    assert "--docker-upgrade" in source
    assert "update --assets --manifest" in source
    assert "CAPSEM_RELEASE_MANIFEST_URL=" in source
    assert 'update --yes' in source
    assert "manifest-origin.json" in source
    assert "snapshot_sha256" in source
    assert "freezes {frozen_manifest_path}" in source


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


def _write_minimal_deb(path: Path, members: dict[str, bytes], *, manifest_url: str) -> None:
    origin = json.dumps(
        {
            "schema": "capsem.manifest_origin.v1",
            "origin": "package",
            "source": manifest_url,
            "package_version": "9.9.9",
        },
        sort_keys=True,
    ).encode()
    members = {
        **members,
        "usr/share/capsem/assets/manifest-origin.json": origin,
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
        with tarfile.open(fileobj=gz, mode="w"):
            pass
    deb = (
        b"!<arch>\n"
        + _ar_member("debian-binary", b"2.0\n")
        + _ar_member("control.tar.gz", control_tar.getvalue())
        + _ar_member("data.tar.gz", data_tar.getvalue())
    )
    path.write_bytes(deb)


def _ar_member(name: str, data: bytes) -> bytes:
    header = (
        f"{name + '/':<16}"
        f"{0:<12}"
        f"{0:<6}"
        f"{0:<6}"
        f"{100644:<8}"
        f"{len(data):<10}`\n"
    ).encode("ascii")
    return header + data + (b"\n" if len(data) % 2 else b"")

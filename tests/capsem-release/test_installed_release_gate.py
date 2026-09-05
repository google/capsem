from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "build_system" / "scripts" / "release" / "verify-installed-release.py"


def _write_fixture(tmp_path: Path) -> tuple[Path, Path, Path]:
    manifest = tmp_path / "served-manifest.json"
    manifest.write_bytes(
        b'{"schema":"capsem.release_graph.v1","profiles":{"code":{},"co-work":{}}}\n'
    )
    home = tmp_path / "home"
    assets = home / "assets"
    assets.mkdir(parents=True)
    (assets / "manifest.json").write_bytes(manifest.read_bytes())
    (assets / "manifest-metadata.json").write_text(
        json.dumps(
            {
                "schema": "capsem.manifest_metadata.v1",
                "manifest_url": manifest.resolve().as_uri(),
                "checked_url": manifest.resolve().as_uri(),
                "channel": "stable",
                "channel_locked": False,
                "package_version": "1.5.9",
                "installed_at": 10,
                "refreshed_at": 11,
                "checked_at": 12,
                "validation_status": "valid",
                "validation_error": None,
                "update_available": False,
            }
        )
        + "\n",
        encoding="utf-8",
    )
    capsem = tmp_path / "capsem"
    capsem.write_text(
        "#!/bin/sh\n"
        "if [ \"${1:-}\" = logs ]; then\n"
        "  cat \"$CAPSEM_RUN_DIR/sessions/$2\"-failed-*/process.log\n"
        "  exit\n"
        "fi\n"
        "cat <<'EOF'\n"
        "Version:   1.5.9\n"
        "Installed: true\n"
        "Running:   true\n"
        "Service:   ok (v1.5.9)\n"
        "Gateway:   ok (port 19222, v1.5.9)\n"
        "Profiles:  2/2 ready (profile)\n"
        f"  source:  {manifest.resolve().as_uri()}\n"
        "  status:  valid\n"
        "EOF\n",
        encoding="utf-8",
    )
    capsem.chmod(0o755)
    return home, manifest, capsem


def _run(
    home: Path,
    manifest: Path,
    capsem: Path,
    *,
    artifact: Path | None = None,
    platform: str = "linux",
    architecture: str = "amd64",
    package_version: str = "1.5.9",
    metadata_manifest_url: str | None = None,
) -> subprocess.CompletedProcess[str]:
    command = [
        sys.executable,
        str(SCRIPT),
        "--capsem",
        str(capsem),
        "--capsem-home",
        str(home),
        "--manifest-url",
        manifest.resolve().as_uri(),
        "--channel",
        "stable",
        "--package-version",
        package_version,
    ]
    if metadata_manifest_url is not None:
        command.extend(["--metadata-manifest-url", metadata_manifest_url])
    if artifact is not None:
        command.extend(
            [
                "--artifact",
                str(artifact),
                "--platform",
                platform,
                "--architecture",
                architecture,
            ]
        )
    return subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


def _add_package(
    home: Path,
    manifest: Path,
    artifact: Path,
    *,
    version: str = "1.5.9",
    platform: str = "linux",
    architecture: str = "x86_64",
) -> None:
    contents = artifact.read_bytes()
    release = json.loads(manifest.read_text(encoding="utf-8"))
    release["packages"] = [
        {
            "name": artifact.name,
            "version": version,
            "platform": platform,
            "architecture": architecture,
            "bytes": len(contents),
            "status": "current",
            "digest": {"sha256": hashlib.sha256(contents).hexdigest()},
        }
    ]
    manifest.write_text(json.dumps(release) + "\n", encoding="utf-8")
    (home / "assets" / "manifest.json").write_bytes(manifest.read_bytes())


def _write_polling_metadata(
    home: Path,
    manifest: Path,
    capsem: Path,
    *,
    checked_url: str | None = None,
    validation_status: str = "valid",
    validation_error: str | None = None,
) -> str:
    polling = "https://release.capsem.org/assets/stable/manifest.json"
    metadata_path = home / "assets" / "manifest-metadata.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata.update(
        manifest_url=polling,
        checked_url=checked_url or polling,
        validation_status=validation_status,
        validation_error=validation_error,
    )
    metadata_path.write_text(json.dumps(metadata) + "\n", encoding="utf-8")
    capsem.write_text(
        capsem.read_text(encoding="utf-8").replace(manifest.resolve().as_uri(), polling),
        encoding="utf-8",
    )
    return polling


def test_installed_release_gate_accepts_exact_manifest_metadata_and_ready_profiles(
    tmp_path: Path,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)

    result = _run(home, manifest, capsem)

    assert result.returncode == 0, result.stderr
    assert "verified installed stable release 1.5.9: 2/2 profiles ready" in result.stdout


def test_installed_release_gate_separates_selected_bytes_from_polling_provenance(
    tmp_path: Path,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    polling = _write_polling_metadata(home, manifest, capsem)

    result = _run(home, manifest, capsem, metadata_manifest_url=polling)

    assert result.returncode == 0, result.stderr


def test_installed_release_gate_accepts_isolated_poll_fetch_error(
    tmp_path: Path,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    polling = _write_polling_metadata(
        home,
        manifest,
        capsem,
        validation_status="fetch_error",
        validation_error=f"error sending request for url ({manifest.resolve().as_uri()})",
    )

    result = _run(home, manifest, capsem, metadata_manifest_url=polling)

    assert result.returncode == 0, result.stderr


def test_installed_release_gate_accepts_selected_bytes_before_public_poll(
    tmp_path: Path,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    polling = _write_polling_metadata(
        home,
        manifest,
        capsem,
        checked_url=manifest.resolve().as_uri(),
    )

    result = _run(home, manifest, capsem, metadata_manifest_url=polling)

    assert result.returncode == 0, result.stderr


def test_installed_release_gate_rejects_fetch_error_without_diagnostics(
    tmp_path: Path,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    polling = _write_polling_metadata(
        home,
        manifest,
        capsem,
        validation_status="fetch_error",
    )

    result = _run(home, manifest, capsem, metadata_manifest_url=polling)

    assert result.returncode != 0
    assert "validation_error" in result.stderr


def test_installed_release_gate_rejects_selected_bytes_fetch_error(
    tmp_path: Path,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    polling = _write_polling_metadata(
        home,
        manifest,
        capsem,
        checked_url=manifest.resolve().as_uri(),
        validation_status="fetch_error",
        validation_error="selected candidate could not be fetched",
    )

    result = _run(home, manifest, capsem, metadata_manifest_url=polling)

    assert result.returncode != 0
    assert "validation_status" in result.stderr


def test_installed_release_gate_rejects_same_source_fetch_error(tmp_path: Path) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    metadata_path = home / "assets" / "manifest-metadata.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata.update(
        validation_status="fetch_error",
        validation_error="candidate could not be fetched",
    )
    metadata_path.write_text(json.dumps(metadata) + "\n", encoding="utf-8")

    result = _run(home, manifest, capsem)

    assert result.returncode != 0
    assert "validation_status" in result.stderr


def test_installed_release_gate_rejects_unknown_checked_url(tmp_path: Path) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    polling = _write_polling_metadata(
        home,
        manifest,
        capsem,
        checked_url="https://mirror.invalid/manifest.json",
    )

    result = _run(home, manifest, capsem, metadata_manifest_url=polling)

    assert result.returncode != 0
    assert "checked_url" in result.stderr


def test_installed_release_gate_rejects_invalid_polling_payload(tmp_path: Path) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    polling = _write_polling_metadata(
        home,
        manifest,
        capsem,
        validation_status="invalid",
        validation_error="candidate signature is invalid",
    )

    result = _run(home, manifest, capsem, metadata_manifest_url=polling)

    assert result.returncode != 0
    assert "validation_status" in result.stderr


def test_installed_release_gate_accepts_manifest_selected_legacy_x86_64_deb(
    tmp_path: Path,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    artifact = tmp_path / "capsem_1.5.9_amd64.deb"
    artifact.write_bytes(b"exact legacy donor package")
    _add_package(home, manifest, artifact)

    result = _run(home, manifest, capsem, artifact=artifact)

    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(
    ("overrides", "field"),
    [
        ({"package_version": "9.9.9"}, "version"),
        ({"platform": "macos"}, "platform"),
        ({"architecture": "x86_64"}, "architecture"),
    ],
)
def test_installed_release_gate_rejects_wrong_expected_package_identity(
    tmp_path: Path,
    overrides: dict[str, str],
    field: str,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    artifact = tmp_path / "capsem_1.5.9_amd64.deb"
    artifact.write_bytes(b"exact legacy donor package")
    _add_package(home, manifest, artifact)

    result = _run(home, manifest, capsem, artifact=artifact, **overrides)

    assert result.returncode != 0
    assert f"manifest-selected package {field}" in result.stderr


def test_installed_release_gate_rejects_rewritten_manifest(tmp_path: Path) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    (home / "assets" / "manifest.json").write_text(
        json.dumps(json.loads(manifest.read_text()), indent=2) + "\n",
        encoding="utf-8",
    )

    result = _run(home, manifest, capsem)

    assert result.returncode != 0
    assert "not byte-for-byte identical" in result.stderr


def test_installed_release_gate_rejects_legacy_sidecars(tmp_path: Path) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    (home / "assets" / "update-check.json").write_text("{}\n", encoding="utf-8")

    result = _run(home, manifest, capsem)

    assert result.returncode != 0
    assert "legacy state path still exists" in result.stderr


def test_installed_release_gate_rejects_partial_profile_readiness(tmp_path: Path) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)
    capsem.write_text(capsem.read_text().replace("Profiles:  2/2", "Profiles:  1/2"))

    result = _run(home, manifest, capsem)

    assert result.returncode != 0
    assert "profiles are not all ready" in result.stderr

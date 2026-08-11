"""Secure pre-activation manifest handoff for exact native package tests."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts" / "pkg-scripts" / "install-manifest"
DIAGNOSTICS = ROOT / "scripts" / "pkg-scripts" / "install-diagnostics"


def _resolve(packaged: str, request: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "bash",
            "-c",
            'source "$1"; capsem_resolve_install_manifest "$2" "$3"',
            "bash",
            str(HELPER),
            packaged,
            str(request),
        ],
        text=True,
        capture_output=True,
        check=False,
    )


def test_secure_request_selects_one_local_serialized_manifest(tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    manifest.write_text('{"channel":"nightly"}\n', encoding="utf-8")
    request = tmp_path / "install-manifest"
    request.write_text(manifest.resolve().as_uri() + "\n", encoding="utf-8")
    request.chmod(0o600)

    result = _resolve("https://release.capsem.org/assets/nightly/manifest.json", request)

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == manifest.resolve().as_uri()


def test_absent_request_preserves_packaged_public_manifest(tmp_path: Path) -> None:
    packaged = "https://release.capsem.org/assets/nightly/manifest.json"

    result = _resolve(packaged, tmp_path / "absent")

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == packaged


def test_secure_pair_preserves_logical_source_and_exact_local_payload(tmp_path: Path) -> None:
    request = tmp_path / "install-manifest"
    payload = tmp_path / "install-manifest.json"
    logical = "http://127.0.0.1:43123/assets/nightly/manifest.json"
    payload.write_text('{"channel":"nightly"}\n', encoding="utf-8")
    payload.chmod(0o600)
    request.write_text(f"{logical}\n{payload.resolve().as_uri()}\n", encoding="utf-8")
    request.chmod(0o600)

    result = _resolve("https://release.capsem.org/assets/stable/manifest.json", request)

    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines() == [logical, payload.resolve().as_uri()]


def test_paired_request_rejects_weak_symlinked_and_orphan_payloads(tmp_path: Path) -> None:
    request = tmp_path / "install-manifest"
    payload = tmp_path / "install-manifest.json"
    logical = "http://127.0.0.1:43123/assets/nightly/manifest.json"
    payload.write_text("{}\n", encoding="utf-8")
    request.write_text(f"{logical}\n{payload.resolve().as_uri()}\n", encoding="utf-8")
    request.chmod(0o600)

    payload.chmod(0o644)
    assert _resolve(logical, request).returncode != 0

    payload.unlink()
    target = tmp_path / "payload-target"
    target.write_text("{}\n", encoding="utf-8")
    target.chmod(0o600)
    os.symlink(target, payload)
    assert _resolve(logical, request).returncode != 0

    request.unlink()
    assert _resolve(logical, request).returncode != 0


def test_request_rejects_remote_bare_multiline_and_weak_mode(tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    manifest.write_text("{}\n", encoding="utf-8")
    request = tmp_path / "install-manifest"
    invalid = (
        "https://attacker.invalid/manifest.json",
        str(manifest),
        f"{manifest.resolve().as_uri()}\n{manifest.resolve().as_uri()}",
    )
    for value in invalid:
        request.write_text(value + "\n", encoding="utf-8")
        request.chmod(0o600)
        result = _resolve("https://release.capsem.org/stable.json", request)
        assert result.returncode != 0

    request.write_text(manifest.resolve().as_uri() + "\n", encoding="utf-8")
    request.chmod(0o644)
    result = _resolve("https://release.capsem.org/stable.json", request)
    assert result.returncode != 0


def test_request_rejects_symlink(tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    manifest.write_text("{}\n", encoding="utf-8")
    target = tmp_path / "request-target"
    target.write_text(manifest.resolve().as_uri() + "\n", encoding="utf-8")
    target.chmod(0o600)
    request = tmp_path / "install-manifest"
    os.symlink(target, request)

    result = _resolve("https://release.capsem.org/stable.json", request)

    assert result.returncode != 0


@pytest.mark.parametrize("exit_status", (0, 7))
def test_shared_failure_trap_always_removes_install_manifest_request(
    tmp_path: Path,
    exit_status: int,
) -> None:
    request = tmp_path / "install-manifest"
    request.write_text("one-shot\n", encoding="utf-8")
    failure_file = tmp_path / "failure.txt"
    run_log = tmp_path / "install.log"

    result = subprocess.run(
        [
            "bash",
            "-c",
            """
set -euo pipefail
source "$1"
CAPSEM_INSTALL_MANIFEST_REQUEST="$2"
CAPSEM_INSTALL_PRESENT_FAILURE=0
CAPSEM_INSTALL_FAILURE_FILE="$3"
CAPSEM_INSTALL_RUN_LOG="$4"
capsem_install_enable_failure_trap
exit "$5"
""",
            "bash",
            str(DIAGNOSTICS),
            str(request),
            str(failure_file),
            str(run_log),
            str(exit_status),
        ],
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == exit_status
    assert request.exists(), "failed postinstall must preserve the handoff for retry"

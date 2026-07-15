from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "verify-installed-release.py"


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


def _run(home: Path, manifest: Path, capsem: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
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
            "1.5.9",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )


def test_installed_release_gate_accepts_exact_manifest_metadata_and_ready_profiles(
    tmp_path: Path,
) -> None:
    home, manifest, capsem = _write_fixture(tmp_path)

    result = _run(home, manifest, capsem)

    assert result.returncode == 0, result.stderr
    assert "verified installed stable release 1.5.9: 2/2 profiles ready" in result.stdout


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

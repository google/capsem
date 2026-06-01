from __future__ import annotations

import os
import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).parent.parent


def _fake_uv(tmp_path: Path) -> tuple[Path, Path]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    log = tmp_path / "uv.log"
    uv = bin_dir / "uv"
    uv.write_text(
        """#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$UV_LOG"
if [[ "$1" == "run" && "$2" == "python3" && "$3" == "-" ]]; then
    assets_dir="${4:?missing assets dir}"
    mkdir -p "$assets_dir"
    cat >/dev/null
    printf '{"version":"test","assets":[]}\\n' > "$assets_dir/manifest.json"
fi
""",
        encoding="utf-8",
    )
    uv.chmod(0o755)
    return bin_dir, log


def test_build_assets_script_routes_profile_builds_through_capsem_admin(
    tmp_path: Path,
) -> None:
    bin_dir, log = _fake_uv(tmp_path)
    profile = tmp_path / "profile.toml"
    profile.write_text('schema = "capsem.profile.v2"\n', encoding="utf-8")
    assets = tmp_path / "assets"
    env = os.environ | {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "UV_LOG": str(log),
    }

    result = subprocess.run(
        [
            "bash",
            "scripts/build-assets.sh",
            "--assets-dir",
            str(assets),
            "--arch",
            "arm64",
            "--profile",
            str(profile),
        ],
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    commands = log.read_text(encoding="utf-8").splitlines()
    assert len(commands) == 3
    assert commands[0].startswith("run capsem-admin image build ")
    assert str(profile) in commands[0]
    assert "--arch arm64" in commands[0]
    assert "--template kernel" in commands[0]
    assert f"--out {assets}/" in commands[0]
    assert commands[1].startswith("run capsem-admin image build ")
    assert "--template rootfs" in commands[1]
    assert "capsem-builder build" not in "\n".join(commands)
    assert commands[2].startswith("run python3 - ")
    assert (assets / "manifest.json").exists()


def test_build_assets_script_repairs_dangling_assets_symlink(
    tmp_path: Path,
) -> None:
    bin_dir, log = _fake_uv(tmp_path)
    profile = tmp_path / "profile.toml"
    profile.write_text('schema = "capsem.profile.v2"\n', encoding="utf-8")
    real_assets = tmp_path / "home" / "assets"
    assets = tmp_path / "assets"
    assets.symlink_to(real_assets)
    env = os.environ | {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "UV_LOG": str(log),
    }

    result = subprocess.run(
        [
            "bash",
            "scripts/build-assets.sh",
            "--assets-dir",
            str(assets),
            "--arch",
            "arm64",
            "--profile",
            str(profile),
        ],
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert real_assets.is_dir()
    assert (real_assets / "manifest.json").exists()


def test_build_assets_script_removes_stale_generated_metadata(tmp_path: Path) -> None:
    bin_dir, log = _fake_uv(tmp_path)
    profile = tmp_path / "profile.toml"
    profile.write_text('schema = "capsem.profile.v2"\n', encoding="utf-8")
    assets = tmp_path / "assets"
    assets.mkdir()
    stale_manifest = assets / "manifest.json"
    stale_manifest.write_text("stale\n", encoding="utf-8")
    stale_manifest.chmod(0o444)
    (assets / "manifest.json.minisig").write_text("stale sig\n", encoding="utf-8")
    (assets / "manifest-sign.dev.pub").write_text("stale pub\n", encoding="utf-8")
    env = os.environ | {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "UV_LOG": str(log),
    }

    result = subprocess.run(
        [
            "bash",
            "scripts/build-assets.sh",
            "--assets-dir",
            str(assets),
            "--arch",
            "x86_64",
            "--profile",
            str(profile),
        ],
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert (assets / "manifest.json").read_text(encoding="utf-8").startswith('{"version"')
    assert not (assets / "manifest.json.minisig").exists()
    assert not (assets / "manifest-sign.dev.pub").exists()


def test_build_assets_script_rejects_unprofiled_builds(
    tmp_path: Path,
) -> None:
    bin_dir, log = _fake_uv(tmp_path)
    env = os.environ | {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "UV_LOG": str(log),
    }

    result = subprocess.run(
        [
            "bash",
            "scripts/build-assets.sh",
            "--arch",
            "x86_64",
        ],
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 1
    assert "--profile is required" in result.stderr
    assert not log.exists()


def test_build_assets_script_rejects_missing_profile(tmp_path: Path) -> None:
    bin_dir, log = _fake_uv(tmp_path)
    env = os.environ | {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "UV_LOG": str(log),
    }

    result = subprocess.run(
        [
            "bash",
            "scripts/build-assets.sh",
            "--profile",
            str(tmp_path / "missing.profile.toml"),
        ],
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 1
    assert "does not exist" in result.stderr
    assert not log.exists()


def test_justfile_exposes_profile_aware_asset_recipes() -> None:
    justfile = (REPO_ROOT / "justfile").read_text(encoding="utf-8")

    assert 'default_asset_profile := "config/profiles/base/coding.profile.toml"' in justfile
    assert "build-kernel arch profile=default_asset_profile" in justfile
    assert "build-rootfs arch profile=default_asset_profile" in justfile
    assert 'build-assets arch="" profile=default_asset_profile' in justfile
    assert 'uv run capsem-admin image build "{{profile}}"' in justfile
    assert 'bash scripts/build-assets.sh --profile "{{profile}}"' in justfile
    assert "capsem-builder build guest/" not in justfile


def test_ensure_service_refreshes_local_profile_after_asset_repack() -> None:
    justfile = (REPO_ROOT / "justfile").read_text(encoding="utf-8")

    assert (
        'CAPSEM_ASSETS_DIR="${CAPSEM_ASSETS_DIR:-$DEV_ASSETS}" {{cli_binary}} setup '
        "--non-interactive --accept-detected"
    ) in justfile

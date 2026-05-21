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


def test_build_assets_script_keeps_lower_level_builder_for_unprofiled_builds(
    tmp_path: Path,
) -> None:
    bin_dir, log = _fake_uv(tmp_path)
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
            "x86_64",
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
    assert commands[0].startswith("run capsem-builder build guest/")
    assert "--arch x86_64" in commands[0]
    assert "--template kernel" in commands[0]
    assert commands[1].startswith("run capsem-builder build guest/")
    assert "--template rootfs" in commands[1]
    assert "capsem-admin image build" not in "\n".join(commands)


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

    assert 'build-kernel arch profile=""' in justfile
    assert 'build-rootfs arch profile=""' in justfile
    assert 'build-assets arch="" profile=""' in justfile
    assert 'uv run capsem-admin image build "{{profile}}"' in justfile
    assert 'profile_args=(--profile "{{profile}}")' in justfile

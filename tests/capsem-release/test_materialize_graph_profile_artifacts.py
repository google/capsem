from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

import blake3


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "materialize-graph-profile-artifacts.py"


def test_materializes_profile_config_from_asset_source_tag(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    config_file = repo / "config" / "profiles" / "code" / "apt-packages.txt"
    config_file.parent.mkdir(parents=True)
    contents = b"zstd\nlz4\n"
    config_file.write_bytes(contents)
    _git(repo, "init")
    _git(repo, "config", "user.email", "test@example.com")
    _git(repo, "config", "user.name", "Test")
    _git(repo, "add", "config/profiles/code/apt-packages.txt")
    _git(repo, "commit", "-m", "asset source")
    _git(repo, "tag", "assets-v2030.0101.1")

    dist = tmp_path / "dist"
    manifest_path = dist / "assets" / "stable" / "manifest.json"
    manifest_path.parent.mkdir(parents=True)
    manifest_path.write_text(
        json.dumps(
            {
                "version": "1.0.1",
                "channel": "stable",
                "packages": [],
                "profiles": {
                    "code": {
                        "id": "code",
                        "revision": "profiles-2030.0101.1",
                        "architectures": [
                            {
                                "architecture": "arm64",
                                "config": [
                                    {
                                        "kind": "apt_packages",
                                        "path": "profiles/code/apt-packages.txt",
                                        "url": "/profiles/releases/profiles-2030.0101.1/code/arm64/apt-packages.txt",
                                        "bytes": len(contents),
                                        "digest": {
                                            "sha256": hashlib.sha256(contents).hexdigest(),
                                            "blake3": blake3.blake3(contents).hexdigest(),
                                        },
                                    }
                                ],
                                "images": [
                                    {
                                        "kind": "kernel",
                                        "url": "https://github.com/google/capsem/releases/download/assets-v2030.0101.1/arm64-vmlinuz",
                                    }
                                ],
                                "evidence": [],
                            }
                        ],
                    }
                },
            }
        ),
        encoding="utf-8",
    )

    subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--dist",
            str(dist),
            "--repo-root",
            str(repo),
            "--channel",
            "stable",
        ],
        check=True,
    )

    artifact = (
        dist
        / "profiles"
        / "releases"
        / "profiles-2030.0101.1"
        / "code"
        / "arm64"
        / "apt-packages.txt"
    )
    assert artifact.read_bytes() == contents


def _git(repo: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(repo), *args], check=True)

from __future__ import annotations

import hashlib
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import blake3
import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "materialize-graph-profile-artifacts.py"


def _module():
    spec = importlib.util.spec_from_file_location("materialize_graph_profiles", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
    _git(repo, "-c", "commit.gpgsign=false", "commit", "-m", "asset source")
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


def test_materializes_profile_config_from_dirty_worktree_source_root(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    config_file = repo / "config" / "profiles" / "code" / "build.sh"
    config_file.parent.mkdir(parents=True)
    config_file.write_bytes(b"#!/bin/sh\necho old\n")
    _git(repo, "init")
    _git(repo, "config", "user.email", "test@example.com")
    _git(repo, "config", "user.name", "Test")
    _git(repo, "add", "config/profiles/code/build.sh")
    _git(repo, "-c", "commit.gpgsign=false", "commit", "-m", "old profile")

    candidate_contents = b"#!/bin/sh\necho candidate\n"
    config_file.write_bytes(candidate_contents)
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
                        "revision": "profiles-candidate",
                        "architectures": [
                            {
                                "architecture": "arm64",
                                "config": [
                                    {
                                        "kind": "build_script",
                                        "path": "profiles/code/build.sh",
                                        "url": "/profiles/releases/profiles-candidate/code/arm64/build.sh",
                                        "bytes": len(candidate_contents),
                                        "digest": {
                                            "sha256": hashlib.sha256(
                                                candidate_contents
                                            ).hexdigest(),
                                            "blake3": blake3.blake3(
                                                candidate_contents
                                            ).hexdigest(),
                                        },
                                    }
                                ],
                                "images": [],
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
            "--source-root",
            str(repo),
        ],
        check=True,
    )

    artifact = (
        dist
        / "profiles"
        / "releases"
        / "profiles-candidate"
        / "code"
        / "arm64"
        / "build.sh"
    )
    assert artifact.read_bytes() == candidate_contents


def test_worktree_source_root_rejects_config_path_escape(tmp_path: Path) -> None:
    module = _module()
    source_root = tmp_path / "source"
    (source_root / "config").mkdir(parents=True)
    (source_root / "secret").write_bytes(b"do not publish")

    with pytest.raises(SystemExit, match="escapes config root"):
        module.read_source(
            repo_root=source_root,
            source_ref=None,
            source_root=source_root,
            source_path="../secret",
        )


def _git(repo: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(repo), *args], check=True)

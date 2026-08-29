from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

import blake3
import pytest
from capsem_builder.release.tools import (
    materialize_graph_profile_artifacts as GRAPH_ARTIFACTS,
)

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "materialize-graph-profile-artifacts.py"


def _module():
    return GRAPH_ARTIFACTS


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
                                        "url": "/profiles/releases/stable/code/profiles-2030.0101.1/arm64/apt-packages.txt",
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
        / "stable"
        / "code"
        / "profiles-2030.0101.1"
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
                                        "url": "/profiles/releases/stable/code/profiles-candidate/arm64/build.sh",
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
        / "stable"
        / "code"
        / "profiles-candidate"
        / "arm64"
        / "build.sh"
    )
    assert artifact.read_bytes() == candidate_contents


def test_same_profile_revision_materializes_independently_per_channel(
    tmp_path: Path,
) -> None:
    contents = b"id = 'code'\n"
    source_root = tmp_path / "source"
    profile_path = source_root / "config" / "profiles" / "code" / "profile.toml"
    profile_path.parent.mkdir(parents=True)
    profile_path.write_bytes(contents)
    dist = tmp_path / "dist"
    for channel in ("stable", "nightly"):
        manifest_path = dist / "assets" / channel / "manifest.json"
        manifest_path.parent.mkdir(parents=True)
        manifest_path.write_text(
            json.dumps(
                {
                    "version": "1.0.1",
                    "channel": channel,
                    "packages": [],
                    "profiles": {
                        "code": {
                            "id": "code",
                            "revision": "same-revision",
                            "architectures": [
                                {
                                    "architecture": "arm64",
                                    "config": [
                                        {
                                            "kind": "profile",
                                            "path": "profiles/code/profile.toml",
                                            "url": (
                                                f"/profiles/releases/{channel}/code/"
                                                "same-revision/arm64/profile.toml"
                                            ),
                                            "bytes": len(contents),
                                            "digest": {
                                                "sha256": hashlib.sha256(
                                                    contents
                                                ).hexdigest(),
                                                "blake3": blake3.blake3(
                                                    contents
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
            str(source_root),
            "--source-root",
            str(source_root),
        ],
        check=True,
    )

    stable = (
        dist
        / "profiles/releases/stable/code/same-revision/arm64/profile.toml"
    )
    nightly = (
        dist
        / "profiles/releases/nightly/code/same-revision/arm64/profile.toml"
    )
    assert stable.read_bytes() == contents
    assert nightly.read_bytes() == contents
    assert stable != nightly


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
            public_base=None,
            source_url="/profiles/releases/stable/code/revision/arm64/secret",
        )


def test_public_mirror_preserves_legacy_profile_config_bytes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    module = _module()
    contents = b"zstd\nlz4\n"
    manifest = {
        "channel": "stable",
        "profiles": {
            "code": {
                "id": "code",
                "revision": "legacy-revision",
                "architectures": [
                    {
                        "architecture": "arm64",
                        "config": [
                            {
                                "path": "profiles/code/apt-packages.txt",
                                "url": (
                                    "/profiles/releases/legacy-revision/"
                                    "arm64/apt-packages.txt"
                                ),
                                "bytes": len(contents),
                                "digest": {
                                    "sha256": hashlib.sha256(contents).hexdigest(),
                                    "blake3": blake3.blake3(contents).hexdigest(),
                                },
                            }
                        ],
                    }
                ],
            }
        },
    }
    requested: list[str] = []

    class Response:
        def __enter__(self):
            return self

        def __exit__(self, *_args: object) -> None:
            return None

        def read(self) -> bytes:
            return contents

    def open_url(request, timeout: int):
        requested.append(request.full_url)
        assert timeout == 60
        return Response()

    monkeypatch.setattr(module, "urlopen", open_url)
    written = module.materialize_manifest_profile_files(
        dist=tmp_path,
        repo_root=tmp_path,
        source_ref=None,
        source_root=None,
        public_base="https://release.example",
        manifest=manifest,
    )

    assert written == 1
    assert requested == [
        (
            "https://release.example/profiles/releases/legacy-revision/"
            "arm64/apt-packages.txt"
        )
    ]
    assert (
        tmp_path / "profiles/releases/legacy-revision/arm64/apt-packages.txt"
    ).read_bytes() == contents


def _git(repo: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(repo), *args], check=True)

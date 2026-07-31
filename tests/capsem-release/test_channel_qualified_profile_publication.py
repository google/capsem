"""Immutable profile addressing is scoped by channel, profile, and revision."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path

import blake3
import pytest

ROOT = Path(__file__).resolve().parents[2]


def _module(name: str, relative: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MATERIALIZE = _module(
    "materialize_channel_qualified_profiles",
    "scripts/materialize-graph-profile-artifacts.py",
)
VERIFY = _module(
    "verify_channel_qualified_profiles",
    "scripts/verify-profile-publication.py",
)


def _descriptor(url: str, contents: bytes) -> dict[str, object]:
    return {
        "kind": "profile",
        "path": "profiles/code/profile.toml",
        "url": url,
        "bytes": len(contents),
        "digest": {
            "sha256": hashlib.sha256(contents).hexdigest(),
            "blake3": blake3.blake3(contents).hexdigest(),
        },
    }


def test_materializer_rejects_revision_profile_path_without_channel(
    tmp_path: Path,
) -> None:
    contents = b"id = 'code'\n"
    source_root = tmp_path / "source"
    profile_path = source_root / "config" / "profiles" / "code" / "profile.toml"
    profile_path.parent.mkdir(parents=True)
    profile_path.write_bytes(contents)
    manifest = {
        "channel": "stable",
        "profiles": {
            "code": {
                "id": "code",
                "revision": "r1",
                "architectures": [
                    {
                        "architecture": "arm64",
                        "config": [
                            _descriptor(
                                "/profiles/releases/r1/code/arm64/profile.toml",
                                contents,
                            )
                        ],
                    }
                ],
            }
        },
    }

    with pytest.raises(SystemExit, match="channel-qualified"):
        MATERIALIZE.materialize_manifest_profile_files(
            dist=tmp_path / "dist",
            repo_root=source_root,
            source_ref=None,
            source_root=source_root,
            manifest=manifest,
        )


def test_publication_verifier_rejects_base_for_another_channel(tmp_path: Path) -> None:
    payload = b"kernel"
    release_dir = tmp_path / "profile-nightly-code-r1"
    release_dir.mkdir()
    (release_dir / "arm64-vmlinuz").write_bytes(payload)
    manifest = {
        "channel": "stable",
        "profiles": {
            "code": {
                "id": "code",
                "revision": "r1",
                "architectures": [
                    {
                        "architecture": "arm64",
                        "config": [],
                        "images": [
                            {
                                **_descriptor(
                                    "https://github.com/google/capsem/releases/download/"
                                    "profile-nightly-code-r1/arm64-vmlinuz",
                                    payload,
                                ),
                                "name": "vmlinuz",
                            }
                        ],
                        "evidence": [],
                    }
                ],
            }
        },
    }
    source = release_dir / "channel-source-stable.json"
    source.write_text(json.dumps(manifest), encoding="utf-8")

    with pytest.raises(ValueError, match="channel/profile/revision identity"):
        VERIFY.verify_profile_publication(
            source,
            "code",
            "https://github.com/google/capsem/releases/download/"
            "profile-nightly-code-r1",
            release_dir,
        )

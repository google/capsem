#!/usr/bin/env python3
"""Materialize graph profile config artifacts from the asset release source ref."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path
from typing import Any

import blake3


ASSET_TAG_RE = re.compile(r"/releases/download/(assets-v[^/]+)/")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Write /profiles/releases config files declared by graph manifests."
    )
    parser.add_argument("--dist", required=True, type=Path)
    parser.add_argument("--repo-root", default=Path("."), type=Path)
    parser.add_argument("--channel", action="append", dest="channels")
    source = parser.add_mutually_exclusive_group()
    source.add_argument(
        "--source-ref",
        help="Override the inferred assets-v... source tag for all profile config files.",
    )
    source.add_argument(
        "--source-root",
        type=Path,
        help="Read profile config files from this local candidate worktree.",
    )
    args = parser.parse_args()

    dist = args.dist.resolve()
    repo_root = args.repo_root.resolve()
    source_root = args.source_root.resolve() if args.source_root else None
    manifests = manifest_paths(dist, args.channels)
    written = 0
    for manifest_path in manifests:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        source_ref = None
        if source_root is None:
            source_ref = args.source_ref or infer_source_ref(manifest, manifest_path)
            ensure_ref(repo_root, source_ref)
        written += materialize_manifest_profile_files(
            dist=dist,
            repo_root=repo_root,
            source_ref=source_ref,
            source_root=source_root,
            manifest=manifest,
        )
    print(f"materialized {written} graph profile artifact files")
    return 0


def manifest_paths(dist: Path, channels: list[str] | None) -> list[Path]:
    if channels:
        paths = [dist / "assets" / channel / "manifest.json" for channel in channels]
    else:
        paths = sorted((dist / "assets").glob("*/manifest.json"))
    missing = [path for path in paths if not path.is_file()]
    if missing:
        joined = ", ".join(str(path) for path in missing)
        raise SystemExit(f"missing channel manifest(s): {joined}")
    return paths


def infer_source_ref(manifest: dict[str, Any], manifest_path: Path) -> str:
    refs: set[str] = set()
    for profile in manifest.get("profiles", {}).values():
        if not isinstance(profile, dict):
            continue
        for architecture in profile.get("architectures", []):
            if not isinstance(architecture, dict):
                continue
            for item in architecture.get("images", []) + architecture.get("evidence", []):
                if not isinstance(item, dict):
                    continue
                url = item.get("url")
                if not isinstance(url, str):
                    continue
                refs.update(ASSET_TAG_RE.findall(url))
    if len(refs) != 1:
        raise SystemExit(
            f"{manifest_path} must reference exactly one assets-v source tag; got {sorted(refs)}"
        )
    return next(iter(refs))


def ensure_ref(repo_root: Path, source_ref: str) -> None:
    result = subprocess.run(
        ["git", "-C", str(repo_root), "rev-parse", "--verify", f"{source_ref}^{{commit}}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if result.returncode == 0:
        return
    subprocess.run(
        [
            "git",
            "-C",
            str(repo_root),
            "fetch",
            "--depth=1",
            "origin",
            f"refs/tags/{source_ref}:refs/tags/{source_ref}",
        ],
        check=True,
    )


def materialize_manifest_profile_files(
    *,
    dist: Path,
    repo_root: Path,
    source_ref: str | None,
    source_root: Path | None,
    manifest: dict[str, Any],
) -> int:
    written = 0
    seen: dict[str, bytes] = {}
    for profile in manifest.get("profiles", {}).values():
        if not isinstance(profile, dict):
            continue
        for architecture in profile.get("architectures", []):
            if not isinstance(architecture, dict):
                continue
            for item in architecture.get("config", []):
                if not isinstance(item, dict):
                    continue
                url = item.get("url")
                source_path = item.get("path")
                if not isinstance(url, str) or not url.startswith("/profiles/releases/"):
                    continue
                if not isinstance(source_path, str) or not source_path:
                    raise SystemExit(f"profile config {url} missing source path")
                source_bytes = read_source(
                    repo_root=repo_root,
                    source_ref=source_ref,
                    source_root=source_root,
                    source_path=source_path,
                )
                verify_descriptor(url, item, source_bytes)
                previous = seen.get(url)
                if previous is not None and previous != source_bytes:
                    raise SystemExit(f"profile config {url} has conflicting source bytes")
                seen[url] = source_bytes
                destination = dist / url.removeprefix("/")
                if destination.exists() and destination.read_bytes() == source_bytes:
                    continue
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(source_bytes)
                written += 1
    return written


def git_show(repo_root: Path, source_ref: str, source_path: str) -> bytes:
    return subprocess.check_output(
        ["git", "-C", str(repo_root), "show", f"{source_ref}:{source_path}"]
    )


def read_source(
    *,
    repo_root: Path,
    source_ref: str | None,
    source_root: Path | None,
    source_path: str,
) -> bytes:
    if source_root is None:
        if source_ref is None:
            raise SystemExit("profile config source needs a git ref or local source root")
        return git_show(repo_root, source_ref, f"config/{source_path}")

    config_root = (source_root / "config").resolve()
    candidate = (config_root / source_path).resolve()
    try:
        candidate.relative_to(config_root)
    except ValueError as error:
        raise SystemExit(f"profile config source escapes config root: {source_path}") from error
    if not candidate.is_file():
        raise SystemExit(f"profile config source is not a file: {source_path}")
    return candidate.read_bytes()


def verify_descriptor(url: str, item: dict[str, Any], contents: bytes) -> None:
    expected_bytes = item.get("bytes")
    if expected_bytes != len(contents):
        raise SystemExit(
            f"profile config {url} size mismatch: expected {expected_bytes}, got {len(contents)}"
        )
    digest = item.get("digest")
    if not isinstance(digest, dict):
        raise SystemExit(f"profile config {url} missing digest")
    expected_sha256 = digest.get("sha256")
    actual_sha256 = hashlib.sha256(contents).hexdigest()
    if expected_sha256 != actual_sha256:
        raise SystemExit(
            f"profile config {url} sha256 mismatch: expected {expected_sha256}, got {actual_sha256}"
        )
    expected_blake3 = digest.get("blake3")
    actual_blake3 = blake3.blake3(contents).hexdigest()
    if expected_blake3 != actual_blake3:
        raise SystemExit(
            f"profile config {url} blake3 mismatch: expected {expected_blake3}, got {actual_blake3}"
        )


if __name__ == "__main__":
    raise SystemExit(main())

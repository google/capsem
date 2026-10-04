"""Verify one immutable runtime release directory against its source manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from urllib.parse import urlparse

import blake3


def verify_runtime_publication(
    manifest_path: Path,
    publication_base: str,
    release_dir: Path,
) -> set[Path]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    runtime = manifest.get("runtime")
    if not isinstance(runtime, dict):
        raise ValueError("source manifest does not contain a runtime")
    channel = manifest.get("channel")
    revision = runtime.get("revision")
    if not isinstance(channel, str) or not channel:
        raise ValueError("source manifest has no channel")
    if not isinstance(revision, str) or not revision:
        raise ValueError("source manifest runtime has no revision")
    parsed = urlparse(publication_base)
    identity = parsed.path.rstrip("/").rsplit("/", maxsplit=1)[-1]
    expected_identity = f"runtime-{channel}-{revision}"
    if identity != expected_identity:
        raise ValueError(
            "immutable runtime publication base does not match the "
            f"channel/revision identity {expected_identity}"
        )
    base = publication_base.rstrip("/") + "/"
    expected: set[Path] = set()
    seen_urls: dict[str, tuple[int, str, str]] = {}
    for architecture in runtime.get("architectures", []):
        if not isinstance(architecture, dict):
            raise ValueError("runtime architecture is malformed")
        arch = architecture.get("architecture")
        if not isinstance(arch, str) or not arch:
            raise ValueError("runtime architecture has no name")
        for section in ("images", "evidence"):
            rows = architecture.get(section)
            if not isinstance(rows, list):
                raise ValueError(f"runtime/{arch} has no {section} array")
            for row in rows:
                if not isinstance(row, dict):
                    raise ValueError(f"runtime/{arch}/{section} row is malformed")
                url = row.get("url")
                if not isinstance(url, str) or not url.startswith(base):
                    raise ValueError(
                        f"runtime/{arch}/{section} URL is outside "
                        f"its immutable publication: {url!r}"
                    )
                expected_bytes = row.get("bytes")
                digest = row.get("digest")
                if not isinstance(expected_bytes, int) or not isinstance(digest, dict):
                    raise ValueError(f"immutable runtime artifact metadata mismatch: {url}")
                identity = (
                    expected_bytes,
                    str(digest.get("sha256")),
                    str(digest.get("blake3")),
                )
                previous_identity = seen_urls.setdefault(url, identity)
                if previous_identity != identity:
                    raise ValueError(
                        f"duplicate immutable runtime URL has conflicting metadata: {url}"
                    )
                name = url.removeprefix(base)
                if (
                    not name
                    or "/" in name
                    or Path(name).name != name
                    or not name.startswith(f"{arch}-")
                ):
                    raise ValueError(f"invalid immutable runtime artifact name: {name!r}")
                path = release_dir / name
                if not path.is_file():
                    raise ValueError(f"immutable runtime artifact is missing: {path}")
                payload = path.read_bytes()
                if expected_bytes != len(payload):
                    raise ValueError(f"immutable runtime artifact metadata mismatch: {name}")
                if digest.get("sha256") != hashlib.sha256(payload).hexdigest():
                    raise ValueError(f"immutable runtime artifact SHA-256 mismatch: {name}")
                if digest.get("blake3") != blake3.blake3(payload).hexdigest():
                    raise ValueError(f"immutable runtime artifact BLAKE3 mismatch: {name}")
                expected.add(path)
        software = architecture.get("software")
        if not isinstance(software, list):
            raise ValueError(f"runtime/{arch} has no software array")
        if software:
            inventory_urls = [
                row.get("url")
                for row in architecture["evidence"]
                if isinstance(row, dict) and row.get("kind") == "software_inventory"
            ]
            if len(inventory_urls) != 1 or not isinstance(inventory_urls[0], str):
                raise ValueError(
                    f"runtime/{arch} software evidence must have exactly "
                    "one manifest-owned software_inventory URL"
                )
            inventory_url = inventory_urls[0]
            for row in software:
                if not isinstance(row, dict) or row.get("evidence") != inventory_url:
                    raise ValueError(
                        f"runtime/{arch} software evidence does not match "
                        f"its manifest-owned software_inventory URL: {row!r}"
                    )
    if not expected:
        raise ValueError("runtime contains no immutable artifacts")
    source_name = f"channel-source-{manifest.get('channel')}.json"
    allowed = expected | {release_dir / source_name}
    actual = {path for path in release_dir.iterdir() if path.is_file()}
    if actual != allowed:
        extra = sorted(str(path) for path in actual - allowed)
        missing = sorted(str(path) for path in allowed - actual)
        raise ValueError(
            f"immutable runtime release file set mismatch: extra={extra}, missing={missing}"
        )
    if parsed.scheme != "https" or not parsed.netloc:
        raise ValueError("immutable runtime publication base must be HTTPS")
    return expected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--publication-base", required=True)
    parser.add_argument("--release-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        verified = verify_runtime_publication(
            args.manifest, args.publication_base, args.release_dir
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"runtime publication verification failed: {error}", file=sys.stderr)
        return 1
    print(f"verified {len(verified)} immutable runtime artifacts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

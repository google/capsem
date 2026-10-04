"""Stage exactly one manifest-described immutable runtime publication."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from urllib.parse import urlparse

from capsem_builder.cache.config import load_policy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.views import ReceiptLocation, copy_view

from . import repository_root

PROJECT_ROOT = repository_root()
CACHE_PATHS = CachePaths(repository_root=PROJECT_ROOT, policy=load_policy(PROJECT_ROOT))


def _safe_source(root: Path, relative: str) -> Path:
    candidate = (root / relative).resolve()
    try:
        candidate.relative_to(root.resolve())
    except ValueError as error:
        raise ValueError(f"runtime publication source escapes {root}: {relative}") from error
    if not candidate.is_file():
        raise ValueError(f"runtime publication source is missing: {candidate}")
    return candidate


def stage_runtime_publication(
    manifest_path: Path,
    assets_dir: Path,
    release_dir: Path,
) -> set[Path]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    runtime = manifest.get("runtime")
    if not isinstance(runtime, dict):
        raise ValueError("source manifest does not contain a runtime")
    if release_dir.exists() and any(release_dir.iterdir()):
        raise ValueError(f"runtime publication directory is not empty: {release_dir}")
    release_dir.mkdir(parents=True, exist_ok=True)
    staged: set[Path] = set()
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
                if not isinstance(url, str):
                    raise ValueError(f"runtime/{arch}/{section} row has no URL")
                destination_name = Path(urlparse(url).path).name
                if not destination_name.startswith(f"{arch}-"):
                    raise ValueError(f"runtime publication URL does not encode architecture: {url}")
                source = _safe_source(assets_dir / arch, destination_name.removeprefix(f"{arch}-"))
                destination = release_dir / destination_name
                if destination in staged:
                    if destination.read_bytes() != source.read_bytes():
                        raise ValueError(
                            f"runtime publication has conflicting bytes for {destination_name}"
                        )
                    continue
                copy_view(
                    CACHE_PATHS,
                    source,
                    destination,
                    receipt_location=ReceiptLocation.INVENTORY,
                )
                staged.add(destination)
    source_name = f"channel-source-{manifest.get('channel')}.json"
    source_destination = release_dir / source_name
    copy_view(
        CACHE_PATHS,
        manifest_path,
        source_destination,
        receipt_location=ReceiptLocation.INVENTORY,
    )
    staged.add(source_destination)
    return staged


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--assets-dir", type=Path, required=True)
    parser.add_argument("--release-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        staged = stage_runtime_publication(args.manifest, args.assets_dir, args.release_dir)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"runtime publication staging failed: {error}", file=sys.stderr)
        return 1
    print(f"staged {len(staged)} immutable runtime publication files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

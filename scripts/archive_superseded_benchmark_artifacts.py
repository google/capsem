#!/usr/bin/env python3
"""Archive superseded benchmark artifacts after a canonical benchmark run."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import zipfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parent.parent
DATA_RE = re.compile(r"^data_(?P<version>.+?)_(?P<arch>x86_64|arm64)(?:_(?P<suffix>.+))?\.json$")
LEGACY_DATA_RE = re.compile(r"^data_(?P<version>.+?)\.json$")


@dataclass(frozen=True)
class BenchmarkArtifact:
    path: Path
    category: str
    lane: tuple[str, str, str]
    sort_key: tuple[float, str, str]
    metadata: dict[str, Any]


def discover_artifacts(root: Path) -> list[BenchmarkArtifact]:
    benchmark_root = root / "benchmarks"
    if not benchmark_root.exists():
        return []

    artifacts = []
    for path in sorted(benchmark_root.glob("*/*.json")):
        if path.parts[-2] == "archive" or not path.name.startswith("data_"):
            continue
        parsed = parse_artifact(path)
        if parsed is not None:
            artifacts.append(parsed)
    return artifacts


def parse_artifact(path: Path) -> BenchmarkArtifact | None:
    category = path.parent.name
    data = read_json(path)
    filename_version, filename_arch, filename_suffix = parse_filename(path.name)
    if filename_version is None and not data:
        return None

    arch = str(data.get("arch") or filename_arch or legacy_arch_for(category))
    version = str(data.get("project_version") or data.get("version") or filename_version or "unknown")
    suffix = lane_suffix(category, filename_suffix, data)
    recorded_at = numeric_timestamp(data.get("recorded_at") or data.get("timestamp"))
    if recorded_at is None:
        recorded_at = path.stat().st_mtime

    return BenchmarkArtifact(
        path=path,
        category=category,
        lane=(category, arch, suffix),
        sort_key=(recorded_at, version, path.name),
        metadata={
            "category": category,
            "arch": arch,
            "project_version": version,
            "suffix": suffix,
            "recorded_at": recorded_at,
            "git_commit": git_commit(data),
        },
    )


def parse_filename(name: str) -> tuple[str | None, str | None, str | None]:
    match = DATA_RE.match(name)
    if match:
        return match.group("version"), match.group("arch"), match.group("suffix")
    match = LEGACY_DATA_RE.match(name)
    if match:
        return match.group("version"), None, None
    return None, None, None


def legacy_arch_for(category: str) -> str:
    # Older macOS lifecycle/fork artifacts predate arch-scoped filenames and are
    # still used as arm64 comparison lanes until macOS reruns the canonical path.
    if category in {"lifecycle", "fork"}:
        return "arm64"
    return "legacy"


def lane_suffix(category: str, filename_suffix: str | None, data: dict[str, Any]) -> str:
    if category == "security-engine":
        return filename_suffix or str(data.get("kind") or "default")
    return "default"


def read_json(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return data if isinstance(data, dict) else {}


def numeric_timestamp(value: Any) -> float | None:
    if isinstance(value, int | float):
        return float(value)
    return None


def git_commit(data: dict[str, Any]) -> str | None:
    git = data.get("git")
    if isinstance(git, dict) and git.get("commit"):
        return str(git["commit"])
    if data.get("source_commit"):
        return str(data["source_commit"])
    return None


def superseded_artifacts(artifacts: list[BenchmarkArtifact]) -> list[BenchmarkArtifact]:
    newest_by_lane: dict[tuple[str, str, str], BenchmarkArtifact] = {}
    for artifact in artifacts:
        current = newest_by_lane.get(artifact.lane)
        if current is None or artifact.sort_key > current.sort_key:
            newest_by_lane[artifact.lane] = artifact
    keep = {artifact.path for artifact in newest_by_lane.values()}
    return [artifact for artifact in artifacts if artifact.path not in keep]


def archive_superseded(
    root: Path,
    *,
    archive_name: str | None = None,
    dry_run: bool = False,
) -> tuple[Path | None, list[BenchmarkArtifact]]:
    artifacts = discover_artifacts(root)
    superseded = superseded_artifacts(artifacts)
    if not superseded:
        return None, []

    archive_dir = root / "benchmarks" / "archive"
    archive_path = archive_dir / (archive_name or default_archive_name())
    if archive_path.suffix != ".zip":
        archive_path = archive_path.with_suffix(".zip")

    if dry_run:
        return archive_path, superseded

    archive_dir.mkdir(parents=True, exist_ok=True)
    manifest = archive_manifest(root, superseded)
    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("MANIFEST.json", json.dumps(manifest, indent=2) + "\n")
        for artifact in superseded:
            zf.write(artifact.path, artifact.path.relative_to(root).as_posix())
    for artifact in superseded:
        artifact.path.unlink()
    return archive_path, superseded


def archive_manifest(root: Path, artifacts: list[BenchmarkArtifact]) -> dict[str, Any]:
    return {
        "schema": "capsem.benchmark-archive.v1",
        "created_at_utc": datetime.now(timezone.utc).isoformat(),
        "policy": "keep newest generated data_*.json per category/arch/lane; zip superseded artifacts",
        "artifacts": [
            {
                "path": artifact.path.relative_to(root).as_posix(),
                "sha256": sha256_file(artifact.path),
                **artifact.metadata,
            }
            for artifact in artifacts
        ],
    }


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def default_archive_name() -> str:
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return f"benchmark-history-{timestamp}.zip"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=PROJECT_ROOT)
    parser.add_argument("--archive-name", help="Archive filename, mostly for tests.")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    archive_path, archived = archive_superseded(
        args.root,
        archive_name=args.archive_name,
        dry_run=args.dry_run,
    )
    if not archived:
        print("No superseded benchmark artifacts to archive.")
        return 0
    action = "Would archive" if args.dry_run else "Archived"
    print(f"{action} {len(archived)} superseded benchmark artifact(s) to {archive_path}")
    for artifact in archived:
        print(f"  {artifact.path.relative_to(args.root)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Resolve Docker rail limits and preserve bounded release-gate diagnostics."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tomllib
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_POLICY = ROOT / "config" / "storage-policy.toml"
POSITIVE_FIELDS = ("minimum_free_gib", "buildkit_keep_gib", "linked_keep_gib")


def load_policy(path: Path) -> dict[str, Any]:
    with path.open("rb") as stream:
        policy = tomllib.load(stream)
    if policy.get("version") != 1:
        raise ValueError(f"unsupported storage policy version: {policy.get('version')!r}")
    docker = policy.get("docker")
    rails = policy.get("rails")
    if not isinstance(docker, dict) or not isinstance(rails, dict) or not rails:
        raise ValueError("storage policy requires [docker] and at least one [rails.*] table")
    for name, rail in rails.items():
        if not isinstance(rail, dict):
            raise ValueError(f"rail {name!r} must be a table")
        for field in POSITIVE_FIELDS:
            value = rail.get(field, docker.get(field))
            if not isinstance(value, int) or value <= 0:
                raise ValueError(f"rail {name!r} requires positive integer {field}")
    return policy


def resolve_rail(policy: dict[str, Any], rail_name: str) -> dict[str, int]:
    rails = policy["rails"]
    if rail_name not in rails:
        raise ValueError(
            f"unknown Docker storage rail {rail_name!r}; expected one of {', '.join(sorted(rails))}"
        )
    defaults = policy["docker"]
    rail = rails[rail_name]
    return {field: int(rail.get(field, defaults[field])) for field in POSITIVE_FIELDS}


def run_text(command: list[str]) -> str:
    result = subprocess.run(command, check=False, capture_output=True, text=True)
    output = (result.stdout or "") + (result.stderr or "")
    return output.strip()


def docker_runtime_report() -> dict[str, Any]:
    capacity = run_text(
        [
            "docker",
            "run",
            "--rm",
            "debian:bookworm-slim",
            "sh",
            "-c",
            "df -Pk / | awk 'NR == 2 { print $2, $3, $4 }'",
        ]
    )
    match = re.search(r"(?m)^(\d+)\s+(\d+)\s+(\d+)$", capacity)
    report: dict[str, Any] = {"available": bool(match)}
    if match:
        total_kib, used_kib, free_kib = (int(value) for value in match.groups())
        report.update(
            {
                "total_gib": round(total_kib / 1024 / 1024, 1),
                "used_gib": round(used_kib / 1024 / 1024, 1),
                "free_gib": round(free_kib / 1024 / 1024, 1),
            }
        )
    return report


def resolved_report(
    policy: dict[str, Any], rail_name: str, *, offline: bool
) -> dict[str, Any]:
    docker_policy = policy["docker"]
    return {
        "policy_version": policy["version"],
        "rail": rail_name,
        "limits": resolve_rail(policy, rail_name),
        "docker": {
            "recommended_disk_gib": docker_policy["recommended_disk_gib"],
            "runtime": {"available": False} if offline else docker_runtime_report(),
        },
        "resources": policy.get("resources", {}),
        "debug_artifacts": policy.get("debug_artifacts", {}),
    }


def command_show(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    report = resolved_report(policy, args.rail, offline=args.offline)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    limits = report["limits"]
    runtime = report["docker"]["runtime"]
    print(f"Docker storage policy v{report['policy_version']} ({args.rail})")
    print(f"  minimum free:  {limits['minimum_free_gib']} GiB")
    print(f"  BuildKit keep: {limits['buildkit_keep_gib']} GiB")
    print(f"  linked keep:   {limits['linked_keep_gib']} GiB")
    print(f"  recommended Docker disk: {report['docker']['recommended_disk_gib']} GiB")
    if runtime.get("available"):
        print(
            "  current Docker disk: "
            f"{runtime['total_gib']} GiB total, {runtime['free_gib']} GiB free"
        )
    return 0


def command_shell(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    limits = resolve_rail(policy, args.rail)
    print(f"CAPSEM_STORAGE_RAIL={args.rail}")
    print(f"CAPSEM_DOCKER_MINIMUM_FREE_GIB={limits['minimum_free_gib']}")
    print(f"CAPSEM_DOCKER_BUILDKIT_KEEP_GIB={limits['buildkit_keep_gib']}")
    print(f"CAPSEM_DOCKER_LINKED_KEEP_GIB={limits['linked_keep_gib']}")
    print(
        "CAPSEM_DOCKER_RECOMMENDED_DISK_GIB="
        f"{int(policy['docker']['recommended_disk_gib'])}"
    )
    return 0


def command_resource(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    resources = policy.get("resources", {})
    if args.name not in resources:
        raise ValueError(f"unknown Docker storage resource: {args.name!r}")
    resource = resources[args.name]
    if args.field not in resource:
        raise ValueError(f"resource {args.name!r} has no field {args.field!r}")
    value = resource[args.field]
    if isinstance(value, (dict, list)):
        print(json.dumps(value, sort_keys=True))
    else:
        print(value)
    return 0


def copy_small_file(source: Path, destination: Path, maximum_bytes: int) -> None:
    try:
        if not source.is_file() or source.stat().st_size > maximum_bytes:
            return
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
    except OSError:
        return


def artifact_tree_size(path: Path) -> int:
    total = 0
    for candidate in path.rglob("*"):
        try:
            if candidate.is_file():
                total += candidate.stat().st_size
        except OSError:
            continue
    return total


def rotate_debug_artifacts(root: Path, debug: dict[str, Any]) -> None:
    try:
        directories = sorted(
            (path for path in root.iterdir() if path.is_dir()),
            key=lambda path: (path.stat().st_mtime, path.name),
        )
    except OSError:
        return
    minimum = int(debug["minimum_runs"])
    maximum = int(debug["maximum_runs"])
    cutoff = datetime.now(timezone.utc).timestamp() - (
        int(debug["maximum_age_days"]) * 24 * 60 * 60
    )
    protected = set(directories[-minimum:]) if minimum > 0 else set()
    stale = list(directories[:-maximum] if maximum > 0 else directories)
    stale.extend(
        path
        for path in directories
        if path not in protected and path.stat().st_mtime < cutoff
    )
    for path in dict.fromkeys(stale):
        shutil.rmtree(path, ignore_errors=True)

    directories = [path for path in directories if path.exists()]
    sizes = {path: artifact_tree_size(path) for path in directories}
    total = sum(sizes.values())
    maximum_bytes = int(debug["maximum_total_gib"]) * 1024**3
    remaining = len(directories)
    for path in directories:
        if total <= maximum_bytes or remaining <= minimum:
            break
        if path in protected:
            continue
        shutil.rmtree(path, ignore_errors=True)
        total -= sizes[path]
        remaining -= 1


def command_capture_failure(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    debug = policy["debug_artifacts"]
    root = ROOT / str(debug["root"])
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    safe_label = re.sub(r"[^A-Za-z0-9_.-]+", "-", args.label).strip("-") or "candidate"
    destination = root / f"{stamp}-storage-{safe_label}"
    destination.mkdir(parents=True, exist_ok=False)

    report = resolved_report(policy, args.rail, offline=args.offline)
    (destination / "policy.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n"
    )
    commands = {
        "docker-system-df.txt": ["docker", "system", "df", "-v"],
        "docker-ps.txt": ["docker", "ps", "-a", "--no-trunc"],
        "docker-images.txt": ["docker", "images", "--digests", "--no-trunc"],
        "docker-buildx-du.txt": ["docker", "buildx", "du"],
    }
    for filename, command in commands.items():
        output = "offline capture: command not executed" if args.offline else run_text(command)
        (destination / filename).write_text(output + "\n")

    maximum_bytes = int(debug["maximum_file_mib"]) * 1024 * 1024
    copy_small_file(ROOT / "target" / "build.log", destination / "build.log", maximum_bytes)
    ironbank = ROOT / "target" / "ironbank-assets"
    for source in ironbank.glob("build-*.log"):
        copy_small_file(source, destination / "ironbank" / source.name, maximum_bytes)
    for source in ironbank.glob("*/run-failure/**/*"):
        if source.name in set(debug["skip_names"]):
            continue
        relative = source.relative_to(ironbank)
        copy_small_file(source, destination / "ironbank" / relative, maximum_bytes)

    rotate_debug_artifacts(root, debug)
    print(f"ARTIFACT: preserved release-gate storage evidence at {destination}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    subparsers = parser.add_subparsers(dest="command", required=True)

    show = subparsers.add_parser("show")
    show.add_argument("--rail", default="default")
    show.add_argument("--offline", action="store_true")
    show.add_argument("--json", action="store_true")

    shell = subparsers.add_parser("shell")
    shell.add_argument("--rail", default="default")

    resource = subparsers.add_parser("resource")
    resource.add_argument("--name", required=True)
    resource.add_argument("--field", required=True)

    capture = subparsers.add_parser("capture-failure")
    capture.add_argument("--rail", default="default")
    capture.add_argument("--label", default="candidate")
    capture.add_argument("--offline", action="store_true")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    try:
        policy = load_policy(args.policy)
        if args.command == "show":
            return command_show(args, policy)
        if args.command == "shell":
            return command_shell(args, policy)
        if args.command == "resource":
            return command_resource(args, policy)
        if args.command == "capture-failure":
            return command_capture_failure(args, policy)
    except (OSError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2
    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())

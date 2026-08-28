#!/usr/bin/env python3
"""Measure and control release-gate Docker storage from one declared policy."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
import tomllib
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_POLICY = ROOT / "config" / "storage-policy.toml"

def _policy_dependencies(root: Path):
    try:
        from capsem_builder.policy import storagepolicycommand, storagepolicyretention
    except ModuleNotFoundError as error:
        if error.name != "capsem_builder":
            raise
        sys.path.insert(0, str(root / "build_system" / "builder"))
        from bootstrap import mount_builder_package

        mount_builder_package(root)
        from capsem_builder.policy import storagepolicycommand, storagepolicyretention
    return storagepolicycommand, storagepolicyretention


_command, _retention = _policy_dependencies(ROOT)
CommandResult = _command.CommandResult
parse_size_bytes = _command.parse_size_bytes
parse_system_df = _command.parse_system_df
parse_volume_sizes = _command.parse_volume_sizes
run_command = _command.run_command
run_text = _command.run_text
cache_violations = _retention.cache_violations
protect_generations = _retention.protect_generations
resource_decision = _retention.resource_decision
superseded_generations = _retention.superseded_generations
validate_bounds = _retention.validate_bounds

POSITIVE_FIELDS = ("minimum_free_gib", "buildkit_keep_gib", "linked_keep_gib")


def utc_now() -> str:
    return datetime.now(UTC).isoformat()


def load_policy(path: Path) -> dict[str, Any]:
    with path.open("rb") as stream:
        policy = tomllib.load(stream)
    if policy.get("version") != 1:
        raise ValueError(f"unsupported storage policy version: {policy.get('version')!r}")
    docker = policy.get("docker")
    rails = policy.get("rails")
    resources = policy.get("resources")
    if not isinstance(docker, dict) or not isinstance(rails, dict) or not rails:
        raise ValueError("storage policy requires [docker] and at least one [rails.*] table")
    for field in ("minimum_disk_gib", "recommended_disk_gib"):
        if not isinstance(docker.get(field), int) or docker[field] <= 0:
            raise ValueError(f"docker policy requires positive integer {field}")
    if docker["recommended_disk_gib"] < docker["minimum_disk_gib"]:
        raise ValueError("recommended Docker disk cannot be smaller than the minimum")
    for name, rail in rails.items():
        if not isinstance(rail, dict):
            raise ValueError(f"rail {name!r} must be a table")
        for field in POSITIVE_FIELDS:
            value = rail.get(field, docker.get(field))
            if not isinstance(value, int) or value <= 0:
                raise ValueError(f"rail {name!r} requires positive integer {field}")
    if not isinstance(resources, dict) or not resources:
        raise ValueError("storage policy requires managed resources")
    tart = policy.get("tart")
    if not isinstance(tart, dict):
        raise ValueError("storage policy requires [tart]")
    for field in ("base_image", "owned_vm_prefix", "report_path"):
        if not isinstance(tart.get(field), str) or not tart[field]:
            raise ValueError(f"Tart policy requires non-empty {field}")
    for name, resource in resources.items():
        if resource.get("kind") not in {"volume", "image"}:
            raise ValueError(f"resource {name!r} requires kind volume or image")
        if resource.get("retention") not in {"cache", "working", "obsolete", "generational"}:
            raise ValueError(f"resource {name!r} requires a declared retention")
        if resource["retention"] == "working" and not (
            resource.get("release_boundary") or resource.get("release_boundaries")
        ):
            raise ValueError(f"working resource {name!r} requires a release boundary")
        if resource["retention"] == "generational":
            # A repository whose tags are minted per content digest, so its
            # docker_name resolves to no single image and `keep_previous`
            # rather than a boundary decides what survives.
            if resource["kind"] != "image":
                raise ValueError(f"generational resource {name!r} must be an image repository")
            keep = resource.get("keep_previous")
            if not isinstance(keep, int) or isinstance(keep, bool) or keep < 0:
                raise ValueError(
                    f"generational resource {name!r} requires an integer keep_previous >= 0"
                )
            validate_bounds(name, resource)
        if not resource.get("reason"):
            raise ValueError(f"resource {name!r} requires a retention reason")
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


def image_generations(repository: str) -> list[dict[str, Any]]:
    """Every tag of a repository, newest first, with exact bytes.

    Inspected one tag at a time rather than in one batched call: a tag that
    disappears between the listing and the inspection shifts a batched
    `--format` result by a line, and the pairing is then silently wrong about
    which image is which. The count here is two or three.
    """
    listed = run_command(
        [
            "docker",
            "image",
            "ls",
            "--filter",
            f"reference={repository}:*",
            "--format",
            "{{.Repository}}:{{.Tag}}",
        ]
    )
    if listed.returncode != 0:
        return []
    generations: list[dict[str, Any]] = []
    for reference in sorted({line.strip() for line in listed.stdout.splitlines() if line.strip()}):
        inspect = run_command(
            [
                "docker",
                "image",
                "inspect",
                reference,
                "--format",
                "{{.Id}}\t{{.Created}}\t{{.Size}}",
            ]
        )
        if inspect.returncode != 0:
            continue
        image_id, created, size = [*inspect.stdout.split("\t"), "", "", ""][:3]
        generations.append(
            {
                "ref": reference,
                "tag": reference.rsplit(":", 1)[-1],
                "id": image_id,
                "created": created,
                # Docker's own number, which double-counts layers shared with
                # another generation. The report's free-space delta is the
                # honest measure of what a removal actually returned.
                "size_bytes": int(size) if size.isdigit() else 0,
            }
        )
    return sorted(generations, key=lambda row: (row["created"], row["ref"]), reverse=True)


def docker_capacity() -> dict[str, Any]:
    result = run_command(
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
    match = re.search(r"(?m)^(\d+)\s+(\d+)\s+(\d+)$", result.stdout)
    if result.returncode != 0 or not match:
        return {"available": False, "error": result.output}
    total_kib, used_kib, free_kib = (int(value) for value in match.groups())
    return {
        "available": True,
        "total_bytes": total_kib * 1024,
        "used_bytes": used_kib * 1024,
        "free_bytes": free_kib * 1024,
    }


def docker_runtime_snapshot(policy: dict[str, Any], *, offline: bool) -> dict[str, Any]:
    resources = policy["resources"]
    if offline:
        managed = {
            name: {
                "kind": resource["kind"],
                "retention": resource["retention"],
                "owner": resource["owner"],
                "present": None,
                "active": None,
                "size_bytes": None,
                "decision": resource_decision(resource),
                "reason": resource["reason"],
                "generations": None,
            }
            for name, resource in sorted(resources.items())
        }
        return {
            "available": False,
            "filesystem": {"available": False},
            "categories": {},
            "managed_resources": managed,
            "unknown_capsem_volumes": [],
        }

    capacity = docker_capacity()
    summary_result = run_command(["docker", "system", "df", "--format", "{{json .}}"])
    verbose_result = run_command(["docker", "system", "df", "-v"])
    if summary_result.returncode != 0 or verbose_result.returncode != 0:
        return {
            "available": False,
            "filesystem": capacity,
            "categories": {},
            "managed_resources": {},
            "unknown_capsem_volumes": [],
            "error": "\n".join(
                part for part in (summary_result.output, verbose_result.output) if part
            ),
        }

    categories = parse_system_df(summary_result.stdout)
    volumes = parse_volume_sizes(verbose_result.stdout)
    managed: dict[str, dict[str, Any]] = {}
    for name, resource in sorted(resources.items()):
        docker_name = str(resource.get("docker_name", name))
        generations: list[dict[str, Any]] = []
        active: bool | None
        if resource["retention"] == "generational":
            generations = image_generations(docker_name)
            present = bool(generations)
            # Left undetermined rather than guessed. Answering it means one
            # `docker ps` per tag on every snapshot, and snapshots bracket
            # every operation; the removal path asks Docker per tag, which is
            # the only place the answer decides anything.
            active = None
            size_bytes = sum(row["size_bytes"] for row in generations)
        elif resource["kind"] == "volume":
            volume = volumes.get(docker_name)
            present = volume is not None
            active = bool(volume and volume["links"])
            size_bytes = volume["size_bytes"] if volume else 0
        else:
            inspect = run_command(
                ["docker", "image", "inspect", docker_name, "--format", "{{.Size}}"]
            )
            present = inspect.returncode == 0 and inspect.stdout.isdigit()
            active_result = run_command(
                ["docker", "ps", "-aq", "--filter", f"ancestor={docker_name}"]
            )
            active = active_result.returncode == 0 and bool(active_result.stdout)
            size_bytes = int(inspect.stdout) if present else 0
        managed[name] = {
            "docker_name": docker_name,
            "kind": resource["kind"],
            "retention": resource["retention"],
            "owner": resource["owner"],
            "present": present,
            "active": active,
            "size_bytes": size_bytes,
            "decision": resource_decision(resource),
            "reason": resource["reason"],
            "generations": generations,
        }

    declared_volumes = {
        str(resource.get("docker_name", name))
        for name, resource in resources.items()
        if resource["kind"] == "volume"
    }
    unknown = [
        {"name": name, **volume}
        for name, volume in sorted(volumes.items())
        if name.startswith("capsem-") and name not in declared_volumes
    ]
    return {
        "available": bool(capacity.get("available")),
        "filesystem": capacity,
        "categories": categories,
        "managed_resources": managed,
        "unknown_capsem_volumes": unknown,
    }


def snapshot_report(
    policy: dict[str, Any],
    rail_name: str,
    *,
    label: str,
    event: str,
    offline: bool,
) -> dict[str, Any]:
    runtime = docker_runtime_snapshot(policy, offline=offline)
    return {
        "schema": "capsem.docker_storage.v1",
        "timestamp": utc_now(),
        "event": event,
        "label": label,
        "rail": rail_name,
        "limits": resolve_rail(policy, rail_name),
        "docker": {
            "minimum_disk_gib": int(policy["docker"]["minimum_disk_gib"]),
            "recommended_disk_gib": int(policy["docker"]["recommended_disk_gib"]),
        },
        "runtime": runtime,
        "resources": runtime["managed_resources"],
    }


def report_path(policy: dict[str, Any]) -> Path:
    override = os.environ.get("CAPSEM_STORAGE_REPORT_PATH")
    return Path(override) if override else ROOT / str(policy["docker"]["report_path"])


def tart_report_path(policy: dict[str, Any]) -> Path:
    override = os.environ.get("CAPSEM_TART_STORAGE_REPORT_PATH")
    return Path(override) if override else ROOT / str(policy["tart"]["report_path"])


def tart_runtime_snapshot(policy: dict[str, Any], *, offline: bool) -> dict[str, Any]:
    tart_policy = policy["tart"]
    if offline:
        return {
            "available": False,
            "cache_allocated_bytes": None,
            "base_image": tart_policy["base_image"],
            "owned_vm_prefix": tart_policy["owned_vm_prefix"],
            "entries": [],
        }
    result = run_command(["tart", "list", "--format", "json"])
    if result.returncode != 0:
        return {
            "available": False,
            "error": result.output,
            "cache_allocated_bytes": None,
            "base_image": tart_policy["base_image"],
            "owned_vm_prefix": tart_policy["owned_vm_prefix"],
            "entries": [],
        }
    raw_entries = json.loads(result.stdout)
    if not isinstance(raw_entries, list):
        raise ValueError("tart list JSON must be an array")
    base_prefix = str(tart_policy["base_image"]).removesuffix(":latest")
    owned_prefix = str(tart_policy["owned_vm_prefix"])
    entries = []
    for raw in raw_entries:
        name = str(raw.get("Name", ""))
        source = str(raw.get("Source", "")).lower()
        if source == "local" and name.startswith(owned_prefix):
            decision = "delete-owned-working-vm"
        elif source == "local":
            decision = "preserve-foreign-local-vm"
        elif name.startswith(base_prefix):
            decision = "retain-base-image-cache"
        else:
            decision = "preserve-unmanaged-oci-cache"
        entries.append(
            {
                "name": name,
                "source": source,
                "state": str(raw.get("State", "")).lower(),
                "running": bool(raw.get("Running", False)),
                "size_gib": raw.get("Size"),
                "disk_gib": raw.get("Disk"),
                "accessed": raw.get("Accessed"),
                "decision": decision,
            }
        )
    tart_home = Path.home() / ".tart"
    usage = run_command(["du", "-sk", str(tart_home)], timeout=60)
    match = re.match(r"^(\d+)", usage.stdout)
    return {
        "available": True,
        "cache_allocated_bytes": int(match.group(1)) * 1024 if match else None,
        "base_image": tart_policy["base_image"],
        "owned_vm_prefix": owned_prefix,
        "entries": entries,
    }


def tart_snapshot_report(
    policy: dict[str, Any], *, label: str, event: str, offline: bool
) -> dict[str, Any]:
    return {
        "schema": "capsem.tart_storage.v1",
        "timestamp": utc_now(),
        "event": event,
        "label": label,
        "runtime": tart_runtime_snapshot(policy, offline=offline),
    }


def append_tart_report(policy: dict[str, Any], report: dict[str, Any]) -> None:
    destination = tart_report_path(policy)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("a") as stream:
        stream.write(json.dumps(report, sort_keys=True) + "\n")


def print_tart_snapshot(report: dict[str, Any]) -> None:
    runtime = report["runtime"]
    print(f"Tart storage [{report['event']}/{report['label']}]")
    if not runtime["available"]:
        print(f"  unavailable ({runtime.get('error', 'offline')})")
        return
    print(f"  ~/.tart allocated: {human_bytes(runtime['cache_allocated_bytes'])}")
    for entry in runtime["entries"]:
        print(
            f"  {entry['name']}: source={entry['source']} state={entry['state']} "
            f"size={entry['size_gib']}GiB decision={entry['decision']}"
        )


def command_tart_snapshot(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    report = tart_snapshot_report(policy, label=args.label, event="snapshot", offline=args.offline)
    if not args.offline:
        append_tart_report(policy, report)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_tart_snapshot(report)
        if not args.offline:
            print(f"  ledger: {tart_report_path(policy)}")
    return 0


def command_tart_clean(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    before = tart_snapshot_report(
        policy, label=f"{args.label}-before", event="snapshot", offline=False
    )
    if not before["runtime"]["available"]:
        print_tart_snapshot(before)
        return 1
    actions = []
    for entry in before["runtime"]["entries"]:
        if entry["decision"] != "delete-owned-working-vm":
            continue
        if entry["running"]:
            actions.append(
                {
                    "target": entry["name"],
                    "status": "retained-running",
                    "reason": "another process may own this running Tart VM",
                }
            )
            continue
        result = run_command(["tart", "delete", entry["name"]], timeout=120)
        actions.append(
            {
                "target": entry["name"],
                "status": "deleted" if result.returncode == 0 else "error",
                "reason": "stopped Capsem-owned working VM has no remaining consumer",
                "output": result.output,
            }
        )
    after = tart_snapshot_report(
        policy, label=f"{args.label}-after", event="snapshot", offline=False
    )
    report = {
        "schema": "capsem.tart_storage.v1",
        "timestamp": utc_now(),
        "event": "clean",
        "label": args.label,
        "before": before["runtime"],
        "actions": actions,
        "after": after["runtime"],
        "ledger": str(tart_report_path(policy)),
    }
    append_tart_report(policy, report)
    print(f"Tart storage control [clean/{args.label}]")
    for action in actions:
        print(f"  {action['status']}: {action['target']} — {action['reason']}")
    owned_after = [
        entry
        for entry in after["runtime"]["entries"]
        if entry["decision"] == "delete-owned-working-vm"
    ]
    print(f"  owned VMs remaining: {len(owned_after)}")
    print(f"  ledger: {tart_report_path(policy)}")
    return 1 if owned_after or any(action["status"] == "error" for action in actions) else 0


def append_report(policy: dict[str, Any], report: dict[str, Any]) -> None:
    destination = report_path(policy)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("a") as stream:
        stream.write(json.dumps(report, sort_keys=True) + "\n")


def human_bytes(value: int | None) -> str:
    if value is None:
        return "unknown"
    for unit, divisor in (("TiB", 1024**4), ("GiB", 1024**3), ("MiB", 1024**2)):
        if value >= divisor:
            return f"{value / divisor:.1f} {unit}"
    return f"{value} B"


def print_snapshot(report: dict[str, Any]) -> None:
    runtime = report["runtime"]
    print(f"Docker storage [{report['event']}/{report['label']}] rail={report['rail']}")
    filesystem = runtime["filesystem"]
    if filesystem.get("available"):
        print(
            "  daemon: "
            f"{human_bytes(filesystem['used_bytes'])} used, "
            f"{human_bytes(filesystem['free_bytes'])} free, "
            f"{human_bytes(filesystem['total_bytes'])} total"
        )
    else:
        print(f"  daemon: unavailable ({filesystem.get('error', 'offline')})")
    for name, row in runtime["categories"].items():
        print(
            f"  {name}: {human_bytes(row['size_bytes'])} "
            f"({human_bytes(row['reclaimable_bytes'])} reclaimable)"
        )
    present = [
        (name, resource) for name, resource in report["resources"].items() if resource["present"]
    ]
    for name, resource in present:
        print(
            f"  {name}: {human_bytes(resource['size_bytes'])} "
            f"{resource['decision']} owner={resource['owner']}"
        )
    for resource in runtime["unknown_capsem_volumes"]:
        print(f"  UNDECLARED {resource['name']}: {human_bytes(resource['size_bytes'])}")


def resolved_report(policy: dict[str, Any], rail_name: str, *, offline: bool) -> dict[str, Any]:
    snapshot = snapshot_report(policy, rail_name, label="policy", event="show", offline=offline)
    return {
        "policy_version": policy["version"],
        "rail": rail_name,
        "limits": snapshot["limits"],
        "docker": {
            **snapshot["docker"],
            "runtime": snapshot["runtime"]["filesystem"],
        },
        "resources": policy.get("resources", {}),
        "tart": {
            "policy": policy["tart"],
            "runtime": tart_runtime_snapshot(policy, offline=offline),
        },
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
    print(f"  minimum Docker disk: {report['docker']['minimum_disk_gib']} GiB")
    print(f"  recommended Docker disk: {report['docker']['recommended_disk_gib']} GiB")
    if runtime.get("available"):
        print(
            "  current Docker disk: "
            f"{human_bytes(runtime['total_bytes'])} total, "
            f"{human_bytes(runtime['free_bytes'])} free"
        )
    return 0


def command_shell(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    limits = resolve_rail(policy, args.rail)
    print(f"CAPSEM_STORAGE_RAIL={args.rail}")
    print(f"CAPSEM_DOCKER_MINIMUM_FREE_GIB={limits['minimum_free_gib']}")
    print(f"CAPSEM_DOCKER_BUILDKIT_KEEP_GIB={limits['buildkit_keep_gib']}")
    print(f"CAPSEM_DOCKER_LINKED_KEEP_GIB={limits['linked_keep_gib']}")
    print(f"CAPSEM_DOCKER_MINIMUM_DISK_GIB={int(policy['docker']['minimum_disk_gib'])}")
    print(f"CAPSEM_DOCKER_RECOMMENDED_DISK_GIB={int(policy['docker']['recommended_disk_gib'])}")
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


def command_snapshot(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    report = snapshot_report(
        policy,
        args.rail,
        label=args.label,
        event="snapshot",
        offline=args.offline,
    )
    if not args.offline:
        append_report(policy, report)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_snapshot(report)
        if not args.offline:
            print(f"  ledger: {report_path(policy)}")
    return 0


def release_boundaries(resource: dict[str, Any]) -> set[str]:
    boundaries = {str(value) for value in resource.get("release_boundaries", [])}
    boundary = resource.get("release_boundary")
    if boundary:
        boundaries.add(str(boundary))
    return boundaries


def remove_resource(name: str, resource: dict[str, Any]) -> dict[str, Any]:
    docker_name = str(resource.get("docker_name", name))
    kind = resource["kind"]
    if kind == "volume":
        inspect = run_command(["docker", "volume", "inspect", docker_name])
        if inspect.returncode != 0:
            return {"target": name, "kind": kind, "status": "absent", "reclaimed_bytes": 0}
        attached = run_command(["docker", "ps", "-aq", "--filter", f"volume={docker_name}"])
        if attached.returncode != 0:
            return {
                "target": name,
                "kind": kind,
                "status": "error",
                "error": attached.output,
                "reclaimed_bytes": 0,
            }
        if attached.stdout:
            return {
                "target": name,
                "kind": kind,
                "status": "retained-active",
                "containers": attached.stdout.splitlines(),
                "reclaimed_bytes": 0,
            }
        result = run_command(["docker", "volume", "rm", docker_name])
    else:
        inspect = run_command(["docker", "image", "inspect", docker_name])
        if inspect.returncode != 0:
            return {"target": name, "kind": kind, "status": "absent", "reclaimed_bytes": 0}
        attached = run_command(["docker", "ps", "-aq", "--filter", f"ancestor={docker_name}"])
        if attached.returncode != 0:
            return {
                "target": name,
                "kind": kind,
                "status": "error",
                "error": attached.output,
                "reclaimed_bytes": 0,
            }
        if attached.stdout:
            return {
                "target": name,
                "kind": kind,
                "status": "retained-active",
                "containers": attached.stdout.splitlines(),
                "reclaimed_bytes": 0,
            }
        result = run_command(["docker", "image", "rm", docker_name])
    return {
        "target": name,
        "docker_name": docker_name,
        "kind": kind,
        "status": "deleted" if result.returncode == 0 else "error",
        "output": result.output,
        "reclaimed_bytes": 0,
    }


def remove_generations(generations: list[dict[str, Any]], *, reason: str) -> list[dict[str, Any]]:
    """Retire tags one at a time, through the guard every other removal uses."""
    actions = []
    for row in generations:
        action = remove_resource(row["ref"], {"kind": "image", "docker_name": row["ref"]})
        action["reason"] = reason
        # Kept apart from `before_bytes`, which `finish_operation` fills from
        # the managed-resource snapshot -- that is keyed by policy name and
        # knows nothing about one tag of a repository.
        action["image_size_bytes"] = row["size_bytes"]
        # `remove_resource` seeds a placeholder zero meaning "not measured".
        # Left in the ledger beside a real deletion it is simply false.
        action.pop("reclaimed_bytes", None)
        actions.append(action)
    return actions


def trim_colima() -> dict[str, Any]:
    if shutil.which("colima") is None:
        return {"status": "skipped", "reason": "colima unavailable"}
    status = run_command(["colima", "status"], timeout=30)
    if status.returncode != 0:
        return {"status": "skipped", "reason": "colima not running"}
    result = run_command(
        [
            "colima",
            "ssh",
            "--",
            "sudo",
            "fstrim",
            "--verbose",
            "/mnt/lima-colima",
        ],
        timeout=120,
    )
    match = re.search(r"([0-9]+) bytes", result.output)
    return {
        "status": "trimmed" if result.returncode == 0 else "error",
        "trimmed_bytes": int(match.group(1)) if match else None,
        "output": result.output,
    }


def operation_report(
    policy: dict[str, Any],
    rail: str,
    *,
    event: str,
    label: str,
    before: dict[str, Any],
    actions: list[dict[str, Any]],
    trim: dict[str, Any],
    after: dict[str, Any],
) -> dict[str, Any]:
    before_free = before["runtime"]["filesystem"].get("free_bytes")
    after_free = after["runtime"]["filesystem"].get("free_bytes")
    reclaimed = (
        after_free - before_free
        if isinstance(before_free, int) and isinstance(after_free, int)
        else None
    )
    return {
        "schema": "capsem.docker_storage.v1",
        "timestamp": utc_now(),
        "event": event,
        "label": label,
        "rail": rail,
        "limits": resolve_rail(policy, rail),
        "before": before["runtime"],
        "actions": actions,
        "fstrim": trim,
        "after": after["runtime"],
        "reclaimed_bytes": reclaimed,
    }


def action_bytes(action: dict[str, Any]) -> str:
    """What an action moved, in the terms that action actually measured.

    A generation removal knows the image's own size and nothing about the
    resource-level before/after, which is keyed by policy name -- so it printed
    `unknown -> unknown; reclaimed 0 B` for a deletion that had just freed
    gigabytes, which reads as a no-op.
    """
    nominal = action.get("image_size_bytes")
    if isinstance(nominal, int):
        # Docker's per-image number, which double-counts layers shared with a
        # surviving generation. The run's free-space delta above is the real
        # figure; this says how big the thing removed was.
        return f" [image {human_bytes(nominal)}]"
    reclaimed = action.get("reclaimed_bytes")
    if isinstance(reclaimed, int):
        return (
            f" [{human_bytes(action.get('before_bytes'))} -> "
            f"{human_bytes(action.get('after_bytes'))}; "
            f"reclaimed {human_bytes(reclaimed)}]"
        )
    return ""


def print_operation(report: dict[str, Any]) -> None:
    before = report["before"]["filesystem"]
    after = report["after"]["filesystem"]
    print(f"Docker storage control [{report['event']}/{report['label']}]")
    if before.get("available") and after.get("available"):
        print(
            f"  free: {human_bytes(before['free_bytes'])} -> "
            f"{human_bytes(after['free_bytes'])} "
            f"(delta {human_bytes(report['reclaimed_bytes'])})"
        )
    for action in report["actions"]:
        print(
            f"  {action['status']}: {action.get('target', action.get('operation'))}"
            + (f" — {action['reason']}" if action.get("reason") else "")
            + action_bytes(action)
        )
    trim = report["fstrim"]
    print(
        f"  fstrim: {trim['status']}"
        + (
            f" ({human_bytes(trim.get('trimmed_bytes'))})"
            if trim.get("trimmed_bytes") is not None
            else f" ({trim.get('reason', trim.get('output', ''))})"
        )
    )
    print(f"  ledger: {report['ledger']}")


def resources_for_boundary(
    policy: dict[str, Any], boundary: str
) -> list[tuple[str, dict[str, Any]]]:
    return [
        (name, resource)
        for name, resource in sorted(policy["resources"].items())
        if resource["retention"] == "working" and boundary in release_boundaries(resource)
    ]


def finish_operation(
    policy: dict[str, Any],
    rail: str,
    *,
    event: str,
    label: str,
    before: dict[str, Any],
    actions: list[dict[str, Any]],
    do_trim: bool,
) -> dict[str, Any]:
    trim = trim_colima() if do_trim else {"status": "skipped", "reason": "no deletion"}
    after = snapshot_report(policy, rail, label=f"{label}-after", event="snapshot", offline=False)
    operation_categories = {
        "buildkit-pressure-prune": "build_cache",
        "buildkit-age-prune": "build_cache",
        "stopped-container-prune": "containers",
        "dangling-image-prune": "images",
    }
    for action in actions:
        target = action.get("target")
        operation = action.get("operation")
        if isinstance(target, str):
            before_resource = before["resources"].get(target, {})
            after_resource = after["resources"].get(target, {})
            action["before_bytes"] = before_resource.get("size_bytes")
            action["after_bytes"] = after_resource.get("size_bytes")
        elif operation == "docker-system-prune-all":
            action["before_bytes"] = sum(
                int(row.get("size_bytes", 0)) for row in before["runtime"]["categories"].values()
            )
            action["after_bytes"] = sum(
                int(row.get("size_bytes", 0)) for row in after["runtime"]["categories"].values()
            )
        elif operation in operation_categories:
            category = operation_categories[operation]
            action["before_bytes"] = (
                before["runtime"]["categories"].get(category, {}).get("size_bytes")
            )
            action["after_bytes"] = (
                after["runtime"]["categories"].get(category, {}).get("size_bytes")
            )
        before_bytes = action.get("before_bytes")
        after_bytes = action.get("after_bytes")
        if isinstance(before_bytes, int) and isinstance(after_bytes, int):
            action["reclaimed_bytes"] = max(0, before_bytes - after_bytes)
    report = operation_report(
        policy,
        rail,
        event=event,
        label=label,
        before=before,
        actions=actions,
        trim=trim,
        after=after,
    )
    report["ledger"] = str(report_path(policy))
    append_report(policy, report)
    print_operation(report)
    return report


def command_release(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    before = snapshot_report(
        policy, args.rail, label=f"{args.boundary}-before", event="snapshot", offline=False
    )
    actions: list[dict[str, Any]] = []
    for name, resource in resources_for_boundary(policy, args.boundary):
        action = remove_resource(name, resource)
        action["reason"] = resource["reason"]
        actions.append(action)
    report = finish_operation(
        policy,
        args.rail,
        event="release",
        label=args.boundary,
        before=before,
        actions=actions,
        do_trim=any(action["status"] == "deleted" for action in actions),
    )
    if any(action["status"] in {"error", "retained-active"} for action in actions):
        return 1
    return 0 if report["after"]["available"] else 1


def command_reclaim(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    """Retire the generations of a content-keyed repository that nothing wants.

    The tag to keep is passed in rather than derived here. `capsem_builder.gate.
    linuxrust.base_tag` computes it from the lockfiles that decide the image's
    contents, and recomputing that hash in this script would be a second source
    of truth for which image the lane is about to run -- the two could disagree,
    and the one holding the delete button would win.
    """
    resources = policy["resources"]
    if args.resource not in resources:
        raise ValueError(f"unknown Docker storage resource: {args.resource!r}")
    resource = resources[args.resource]
    if resource["retention"] != "generational":
        raise ValueError(
            f"resource {args.resource!r} is {resource['retention']!r}, not generational: "
            "only a repository whose tags are minted per content digest is reclaimed this way"
        )
    repository = str(resource.get("docker_name", args.resource))
    if args.keep.rsplit(":", 1)[0] != repository:
        raise ValueError(f"{args.keep!r} is not a tag of {repository!r}")
    protected = {str(tag) for tag in getattr(args, "protect", ())}
    invalid = sorted(tag for tag in protected if tag.rsplit(":", 1)[0] != repository)
    if invalid:
        raise ValueError(f"protected images are not tags of {repository!r}: {', '.join(invalid)}")

    generations = image_generations(repository)
    if not any(row["ref"] == args.keep for row in generations):
        # The one outcome that is never right. The caller has just built or
        # confirmed this tag, so its absence means something else removed it --
        # most likely a concurrent worktree, whose run is about to need it.
        print(
            f"ERROR: {args.keep} is not present, so there is no anchor to reclaim "
            f"around; refusing to retire every generation of {repository}.",
            file=sys.stderr,
        )
        return 1

    before = snapshot_report(
        policy, args.rail, label=f"reclaim-{args.resource}-before", event="snapshot", offline=False
    )
    retained, removable = superseded_generations(
        generations, keep=args.keep, keep_previous=int(resource["keep_previous"])
    )
    retained, removable = protect_generations(retained, removable, protected)
    actions: list[dict[str, Any]] = [
        {
            "target": row["ref"],
            "kind": "image",
            "status": "retained-generation",
            "reason": (
                "pinned by an active or resumable product receipt"
                if row["ref"] in protected
                else "kept as a previous generation, so a revert does not rebuild"
            ),
            "image_size_bytes": row["size_bytes"],
            "reclaimed_bytes": 0,
        }
        for row in retained
    ]
    actions += remove_generations(
        removable,
        reason=f"superseded by {args.keep}, which is what the current lockfiles resolve to",
    )
    survivors = [row for row in generations if row["ref"] == args.keep or row in retained]
    violations = cache_violations(resource, survivors, keep=args.keep)
    if violations:
        actions.append(
            {
                "target": args.resource,
                "kind": "image-cache",
                "status": "error",
                "reason": "cannot satisfy cache policy without deleting pinned images: "
                + "; ".join(violations),
                "reclaimed_bytes": 0,
            }
        )
    report = finish_operation(
        policy,
        args.rail,
        event="reclaim",
        label=args.resource,
        before=before,
        actions=actions,
        do_trim=any(action["status"] == "deleted" for action in actions),
    )
    return (
        1
        if any(action["status"] in {"error", "retained-active"} for action in actions)
        else (0 if report["after"]["available"] else 1)
    )


def remove_obsolete(policy: dict[str, Any]) -> list[dict[str, Any]]:
    actions = []
    for name, resource in sorted(policy["resources"].items()):
        if resource["retention"] != "obsolete":
            continue
        action = remove_resource(name, resource)
        action["reason"] = resource["reason"]
        actions.append(action)
    return actions


def command_enforce(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    before = snapshot_report(
        policy, args.rail, label=f"{args.label}-before", event="snapshot", offline=False
    )
    if not before["runtime"]["available"]:
        print_snapshot(before)
        return 1
    actions = remove_obsolete(policy)
    current = snapshot_report(
        policy, args.rail, label=f"{args.label}-measured", event="snapshot", offline=False
    )
    minimum_bytes = resolve_rail(policy, args.rail)["minimum_free_gib"] * 1024**3
    # The re-measure after pruning gets the same availability guard as the
    # opening snapshot. A daemon that stops answering between the two -- which
    # pruning itself can provoke -- yields {"available": False} with no
    # free_bytes, and dereferencing it raised a bare KeyError that named
    # neither Docker nor the gate it stopped.
    filesystem = current["runtime"]["filesystem"]
    if not filesystem.get("available"):
        print_snapshot(current)
        print(
            "ERROR: Docker did not report filesystem capacity after pruning, so "
            "free space cannot be checked against the "
            f"{human_bytes(minimum_bytes)} floor: "
            f"{filesystem.get('error', 'no error reported')}",
            file=sys.stderr,
        )
        return 1
    free_bytes = filesystem["free_bytes"]
    if free_bytes < minimum_bytes:
        keep_gib = resolve_rail(policy, args.rail)["buildkit_keep_gib"]
        prune = run_command(
            [
                "docker",
                "builder",
                "prune",
                "--force",
                "--keep-storage",
                f"{keep_gib}GB",
            ],
            timeout=600,
        )
        actions.append(
            {
                "operation": "buildkit-pressure-prune",
                "status": "completed" if prune.returncode == 0 else "error",
                "reason": (
                    f"daemon free space was below {human_bytes(minimum_bytes)}; "
                    f"retain {keep_gib} GiB of hottest reusable layers"
                ),
                "output": prune.output,
            }
        )
    report = finish_operation(
        policy,
        args.rail,
        event="enforce",
        label=args.label,
        before=before,
        actions=actions,
        do_trim=any(action["status"] in {"deleted", "completed"} for action in actions),
    )
    after = report["after"]["filesystem"]
    if not after.get("available") or after["free_bytes"] < minimum_bytes:
        print(
            f"ERROR: Docker rail {args.rail!r} requires "
            f"{human_bytes(minimum_bytes)} free; "
            f"{human_bytes(after.get('free_bytes'))} remains.",
            file=sys.stderr,
        )
        return 1
    return 0


def prune_action(
    operation: str, command: list[str], reason: str, *, timeout: int = 600
) -> dict[str, Any]:
    result = run_command(command, timeout=timeout)
    return {
        "operation": operation,
        "status": "completed" if result.returncode == 0 else "error",
        "reason": reason,
        "output": result.output,
    }


def command_gc(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    before = snapshot_report(policy, args.rail, label="gc-before", event="snapshot", offline=False)
    docker = policy["docker"]
    container_age = int(docker["container_max_age_hours"])
    cache_age = int(docker["cache_max_age_hours"])
    keep_gib = resolve_rail(policy, args.rail)["buildkit_keep_gib"]
    actions = remove_obsolete(policy)
    actions.extend(
        [
            prune_action(
                "stopped-container-prune",
                [
                    "docker",
                    "container",
                    "prune",
                    "--force",
                    "--filter",
                    f"until={container_age}h",
                ],
                f"stopped containers older than {container_age}h have no consumer",
            ),
            prune_action(
                "dangling-image-prune",
                [
                    "docker",
                    "image",
                    "prune",
                    "--force",
                    "--filter",
                    f"until={cache_age}h",
                ],
                f"untagged images older than {cache_age}h have no named consumer",
            ),
            prune_action(
                "buildkit-age-prune",
                [
                    "docker",
                    "builder",
                    "prune",
                    "--force",
                    "--filter",
                    f"until={cache_age}h",
                    "--keep-storage",
                    f"{keep_gib}GB",
                ],
                (
                    f"unused layers older than {cache_age}h are evictable while "
                    f"{keep_gib} GiB of the hot graph remains"
                ),
            ),
        ]
    )
    report = finish_operation(
        policy,
        args.rail,
        event="gc",
        label="routine",
        before=before,
        actions=actions,
        do_trim=True,
    )
    return (
        1
        if any(action["status"] == "error" for action in actions)
        else (0 if report["after"]["available"] else 1)
    )


def command_clean(args: argparse.Namespace, policy: dict[str, Any]) -> int:
    before = snapshot_report(
        policy, args.rail, label=f"clean-{args.scope}-before", event="snapshot", offline=False
    )
    actions = []
    for name, resource in sorted(policy["resources"].items()):
        if args.scope == "working" and resource["retention"] == "cache":
            continue
        if resource["retention"] == "generational":
            # Every generation, including the current one: a clean is a cold
            # rebuild, and `remove_resource` would look for a `:latest` tag
            # this repository never has and report the whole thing absent.
            actions += remove_generations(
                image_generations(str(resource.get("docker_name", name))),
                reason=f"explicit clean scope={args.scope}; {resource['reason']}",
            )
            continue
        action = remove_resource(name, resource)
        action["reason"] = f"explicit clean scope={args.scope}; {resource['reason']}"
        actions.append(action)
    if args.scope == "all":
        actions.append(
            prune_action(
                "docker-system-prune-all",
                ["docker", "system", "prune", "--all", "--force", "--volumes"],
                "explicit clean all removes every unused Docker object and cache",
            )
        )
    report = finish_operation(
        policy,
        args.rail,
        event="clean",
        label=args.scope,
        before=before,
        actions=actions,
        do_trim=any(action["status"] == "deleted" for action in actions),
    )
    if any(action["status"] in {"error", "retained-active"} for action in actions):
        return 1
    return 0 if report["after"]["available"] else 1


def copy_small_file(source: Path, destination: Path, maximum_bytes: int) -> str:
    """Copy if it is there and small enough, and say which of those it was.

    Returning `None` for three different outcomes -- absent, over the cap,
    unreadable -- is what made a preserved bundle unable to distinguish "there
    was nothing to collect" from "the collector failed". The caller records
    the answer, so the gap is visible in the bundle rather than in the
    post-mortem that needed it.

    Oversized files are tailed, not dropped. The first bundle written with
    this manifest showed `build.log` and `docker-storage.jsonl` as
    `too-large`, meaning every previous bundle had silently omitted both.
    """
    try:
        if not source.is_file():
            return "absent"
        destination.parent.mkdir(parents=True, exist_ok=True)
        if source.stat().st_size > maximum_bytes:
            # Keep the tail rather than nothing. A build log that overran the
            # cap was silently discarded, which removed the most useful file
            # in the bundle exactly when the run was long enough to need it --
            # and the end of a log is where the failure is.
            with source.open("rb") as handle:
                handle.seek(-maximum_bytes, os.SEEK_END)
                destination.write_bytes(handle.read())
            return "truncated"
        shutil.copy2(source, destination)
    except OSError:
        return "unreadable"
    return "copied"


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
    cutoff = datetime.now(UTC).timestamp() - (int(debug["maximum_age_days"]) * 24 * 60 * 60)
    protected = set(directories[-minimum:]) if minimum > 0 else set()
    stale = list(directories[:-maximum] if maximum > 0 else directories)
    stale.extend(
        path for path in directories if path not in protected and path.stat().st_mtime < cutoff
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
    stamp = datetime.now(UTC).strftime("%Y%m%d-%H%M%S")
    safe_label = re.sub(r"[^A-Za-z0-9_.-]+", "-", args.label).strip("-") or "candidate"
    if args.run_id is not None and not re.fullmatch(r"[A-Za-z0-9_.-]{1,160}", args.run_id):
        raise ValueError("failure capture run id is not canonical")
    if args.source_commit is not None and not re.fullmatch(r"[0-9a-f]{40}", args.source_commit):
        raise ValueError("failure capture source commit is not canonical")
    destination = root / f"{stamp}-storage-{safe_label}"
    destination.mkdir(parents=True, exist_ok=False)

    report = resolved_report(policy, args.rail, offline=args.offline)
    (destination / "policy.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    commands = {
        "docker-system-df.txt": ["docker", "system", "df", "-v"],
        "docker-ps.txt": ["docker", "ps", "-a", "--no-trunc"],
        "docker-images.txt": ["docker", "images", "--digests", "--no-trunc"],
        "docker-buildx-du.txt": ["docker", "buildx", "du"],
    }
    for filename, command in commands.items():
        output = "offline capture: command not executed" if args.offline else run_text(command)
        (destination / filename).write_text(output + "\n")
    tart_report = tart_snapshot_report(
        policy, label=args.label, event="failure-capture", offline=args.offline
    )
    (destination / "tart-storage.json").write_text(
        json.dumps(tart_report, indent=2, sort_keys=True) + "\n"
    )

    maximum_bytes = int(debug["maximum_file_mib"]) * 1024 * 1024
    collected: list[dict[str, str]] = []

    def collect(source: Path, target: Path) -> None:
        outcome = copy_small_file(source, target, maximum_bytes)
        collected.append({"source": str(source), "outcome": outcome})

    collect(ROOT / "target" / "build.log", destination / "build.log")
    collect(report_path(policy), destination / "docker-storage.jsonl")
    collect(ROOT / "target" / "storage" / "host-cleanup.jsonl", destination / "host-cleanup.jsonl")

    ironbank = ROOT / "target" / "ironbank-assets"
    # Named even when the glob is empty. A build that never ran and a
    # collector that never looked produce the same silence otherwise, and the
    # difference is the whole point of preserving evidence.
    build_logs = sorted(ironbank.glob("build-*.log"))
    if not build_logs:
        collected.append({"source": str(ironbank / "build-*.log"), "outcome": "absent"})
    for source in build_logs:
        collect(source, destination / "ironbank" / source.name)

    failures = [
        source
        for source in sorted(ironbank.glob("*/run-failure/**/*"))
        if source.name not in set(debug["skip_names"])
    ]
    if not failures:
        collected.append({"source": str(ironbank / "*/run-failure"), "outcome": "absent"})
    for source in failures:
        collect(source, destination / "ironbank" / source.relative_to(ironbank))

    (destination / "collected.json").write_text(
        json.dumps(
            {
                "files": collected,
                "run_id": args.run_id,
                "source_commit": args.source_commit,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )

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

    snapshot = subparsers.add_parser("snapshot")
    snapshot.add_argument("--rail", default="default")
    snapshot.add_argument("--label", default="manual")
    snapshot.add_argument("--offline", action="store_true")
    snapshot.add_argument("--json", action="store_true")

    tart_snapshot = subparsers.add_parser("tart-snapshot")
    tart_snapshot.add_argument("--label", default="manual")
    tart_snapshot.add_argument("--offline", action="store_true")
    tart_snapshot.add_argument("--json", action="store_true")

    tart_clean = subparsers.add_parser("tart-clean")
    tart_clean.add_argument("--label", default="boundary")

    enforce = subparsers.add_parser("enforce")
    enforce.add_argument("--rail", default="default")
    enforce.add_argument("--label", default="preflight")

    release = subparsers.add_parser("release")
    release.add_argument("--rail", default="default")
    release.add_argument("--boundary", required=True)

    reclaim = subparsers.add_parser("reclaim")
    reclaim.add_argument("--rail", default="default")
    reclaim.add_argument("--resource", required=True)
    reclaim.add_argument("--keep", required=True)
    reclaim.add_argument("--protect", action="append", default=[])

    gc = subparsers.add_parser("gc")
    gc.add_argument("--rail", default="default")

    clean = subparsers.add_parser("clean")
    clean.add_argument("--rail", default="default")
    clean.add_argument("--scope", choices=("working", "all"), default="working")

    capture = subparsers.add_parser("capture-failure")
    capture.add_argument("--rail", default="default")
    capture.add_argument("--label", default="candidate")
    capture.add_argument("--run-id")
    capture.add_argument("--source-commit")
    capture.add_argument("--offline", action="store_true")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    try:
        policy = load_policy(args.policy)
        commands = {
            "show": command_show,
            "shell": command_shell,
            "resource": command_resource,
            "snapshot": command_snapshot,
            "tart-snapshot": command_tart_snapshot,
            "tart-clean": command_tart_clean,
            "enforce": command_enforce,
            "release": command_release,
            "reclaim": command_reclaim,
            "gc": command_gc,
            "clean": command_clean,
            "capture-failure": command_capture_failure,
        }
        return commands[args.command](args, policy)
    except (json.JSONDecodeError, OSError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

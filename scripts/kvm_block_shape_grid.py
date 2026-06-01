#!/usr/bin/env python3
"""Run focused KVM virtio-blk shape gridsearches through Capsem."""

from __future__ import annotations

import argparse
import itertools
import json
import os
import platform
import re
import subprocess
import sys
import time
from pathlib import Path

from hypervisor_bench_common import ROOT, git_commit, host_metadata, project_version


ARTIFACT_DIR = ROOT / "benchmarks" / "kvm-block-shape"
TARGET_HOME = ROOT / "target" / "kvm-block-shape-grid" / "home"


def parse_csv_u16(raw: str, *, name: str) -> list[int]:
    values: list[int] = []
    for part in raw.split(","):
        part = part.strip()
        if not part:
            continue
        value = int(part, 10)
        if value <= 0 or value > 65535:
            raise argparse.ArgumentTypeError(f"{name} value out of u16 range: {value}")
        values.append(value)
    if not values:
        raise argparse.ArgumentTypeError(f"{name} must contain at least one value")
    return values


def parse_seg_maxes(raw: str, queue_size: int) -> list[int]:
    values: list[int] = []
    for part in raw.split(","):
        part = part.strip().lower()
        if not part:
            continue
        if part == "auto":
            value = queue_size - 2
        else:
            value = int(part, 10)
        if value <= 0 or value > queue_size - 2:
            continue
        if value not in values:
            values.append(value)
    return values


def extract_json(output: str, name: str) -> dict:
    pattern = re.compile(
        rf"CAPSEM_KVM_SHAPE_{re.escape(name)}_JSON_BEGIN\s*(\{{.*?\}})\s*"
        rf"CAPSEM_KVM_SHAPE_{re.escape(name)}_JSON_END",
        re.DOTALL,
    )
    match = pattern.search(output)
    if not match:
        raise RuntimeError(f"missing {name} JSON marker")
    return json.loads(match.group(1))


def extract_sysfs(output: str) -> dict[str, str]:
    pattern = re.compile(
        r"CAPSEM_KVM_SHAPE_SYSFS_BEGIN\s*(.*?)\s*CAPSEM_KVM_SHAPE_SYSFS_END",
        re.DOTALL,
    )
    match = pattern.search(output)
    if not match:
        return {}
    sysfs: dict[str, str] = {}
    for line in match.group(1).splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            sysfs[key.strip()] = value.strip()
    return sysfs


def guest_command(*, startup: bool) -> str:
    parts = [
        "set -eu",
        "echo CAPSEM_KVM_SHAPE_SYSFS_BEGIN",
        "printf 'mq_dirs='; ls /sys/block/vda/mq 2>/dev/null | wc -l",
        "printf 'max_segments='; cat /sys/block/vda/queue/max_segments",
        "printf 'logical_block_size='; cat /sys/block/vda/queue/logical_block_size",
        "printf 'nr_requests='; cat /sys/block/vda/queue/nr_requests",
        "echo CAPSEM_KVM_SHAPE_SYSFS_END",
        "capsem-bench rootfs >/dev/null",
        "echo CAPSEM_KVM_SHAPE_ROOTFS_JSON_BEGIN",
        "cat /tmp/capsem-benchmark.json",
        "echo CAPSEM_KVM_SHAPE_ROOTFS_JSON_END",
    ]
    if startup:
        parts.extend(
            [
                "capsem-bench startup >/dev/null",
                "echo CAPSEM_KVM_SHAPE_STARTUP_JSON_BEGIN",
                "cat /tmp/capsem-benchmark.json",
                "echo CAPSEM_KVM_SHAPE_STARTUP_JSON_END",
            ]
        )
    return "; ".join(parts)


def run_shape(shape: dict[str, int], *, startup: bool, timeout: int) -> dict:
    env = {
        **os.environ,
        "CAPSEM_HOME": str(TARGET_HOME),
        "CAPSEM_RUN_DIR": str(TARGET_HOME / "run"),
        "CAPSEM_ASSETS_DIR": str(ROOT / "assets"),
        "CAPSEM_KVM_BLK_QUEUE_COUNT": str(shape["queue_count"]),
        "CAPSEM_KVM_BLK_QUEUE_SIZE": str(shape["queue_size"]),
        "CAPSEM_KVM_BLK_SEG_MAX": str(shape["seg_max"]),
        "CAPSEM_KVM_BLK_LOGICAL_BLOCK_SIZE": str(shape["logical_block_size"]),
    }
    started = time.time()
    proc = subprocess.run(
        ["just", "exec", guest_command(startup=startup)],
        cwd=ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
    )
    duration = time.time() - started
    combined = proc.stdout + "\n" + proc.stderr
    result: dict = {
        "shape": shape,
        "returncode": proc.returncode,
        "duration_s": round(duration, 3),
        "sysfs": extract_sysfs(combined),
    }
    if proc.returncode == 0:
        result["rootfs"] = extract_json(combined, "ROOTFS")["rootfs"]
        if startup:
            result["startup"] = extract_json(combined, "STARTUP")["startup"]
    else:
        result["error_tail"] = combined[-4000:]
    return result


def build_shapes(args: argparse.Namespace) -> list[dict[str, int]]:
    queue_counts = parse_csv_u16(args.queue_counts, name="queue-counts")
    queue_sizes = parse_csv_u16(args.queue_sizes, name="queue-sizes")
    logical_block_sizes = parse_csv_u16(args.logical_block_sizes, name="logical-block-sizes")
    shapes: list[dict[str, int]] = []
    for queue_count, queue_size, logical_block_size in itertools.product(
        queue_counts, queue_sizes, logical_block_sizes
    ):
        for seg_max in parse_seg_maxes(args.seg_maxes, queue_size):
            shapes.append(
                {
                    "queue_count": queue_count,
                    "queue_size": queue_size,
                    "seg_max": seg_max,
                    "logical_block_size": logical_block_size,
                }
            )
    if args.limit is not None:
        shapes = shapes[: args.limit]
    return shapes


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--queue-counts", default="1,2,4,8,16")
    parser.add_argument("--queue-sizes", default="128,256,512")
    parser.add_argument("--seg-maxes", default="auto,64")
    parser.add_argument("--logical-block-sizes", default="512,4096")
    parser.add_argument("--startup", action="store_true", help="also run capsem-bench startup")
    parser.add_argument("--timeout", type=int, default=420)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    shapes = build_shapes(args)
    if args.dry_run:
        print(json.dumps({"count": len(shapes), "shapes": shapes}, indent=2))
        return 0

    artifact = {
        "schema": "capsem.kvm-block-shape-grid.v1",
        "timestamp": time.time(),
        "version": project_version(),
        "arch": platform.machine(),
        "git_commit": git_commit(),
        "host": host_metadata(),
        "startup": args.startup,
        "shapes": shapes,
        "results": [],
    }
    for index, shape in enumerate(shapes, start=1):
        print(f"[{index}/{len(shapes)}] {shape}", flush=True)
        result = run_shape(shape, startup=args.startup, timeout=args.timeout)
        artifact["results"].append(result)
        if result["returncode"] != 0:
            print(f"  failed: returncode={result['returncode']}", file=sys.stderr, flush=True)
        else:
            rootfs = result["rootfs"]
            print(
                "  rootfs: "
                f"rand={rootfs['rand_read_4k']['iops']:.0f} iops "
                f"small_js={rootfs['small_js_read']['ops_per_sec']:.0f}/s "
                f"meta={rootfs['metadata_stat']['stats_per_sec']:.0f}/s",
                flush=True,
            )

    ARTIFACT_DIR.mkdir(parents=True, exist_ok=True)
    out = ARTIFACT_DIR / f"data_{project_version()}_{platform.machine()}_{int(time.time())}.json"
    out.write_text(json.dumps(artifact, indent=2) + "\n")
    print(f"wrote {out}")
    return 0 if all(r["returncode"] == 0 for r in artifact["results"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())

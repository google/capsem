#!/usr/bin/env python3
"""Run Capsem rootfs/startup benchmarks under a private crosvm reference build."""

from __future__ import annotations

import argparse
import json
import platform
import subprocess
import sys
import time
from pathlib import Path

from hypervisor_bench_common import (
    ROOT,
    build_initrd,
    extract_json,
    git_commit,
    host_metadata,
    project_version,
    run,
)


TARGET = ROOT / "target" / "crosvm-bench"
CROSVM_ROOT = ROOT / "private" / "crosvm"
CROSVM = CROSVM_ROOT / "target" / "release" / "crosvm"


def ensure_crosvm() -> dict[str, str | None]:
    if not CROSVM.exists():
        raise RuntimeError(
            "crosvm reference binary is missing. Build it under private/crosvm "
            "with: cargo build --release --no-default-features --features default-no-sandbox"
        )
    version = run([str(CROSVM), "version"], check=False)
    return {
        "binary": str(CROSVM),
        "version": version.stdout.strip().splitlines()[0] if version.stdout.strip() else None,
        "source_commit": git_commit(CROSVM_ROOT),
    }


def build_crosvm_initrd(work: Path) -> Path:
    return build_initrd(
        work,
        ROOT / "assets" / "x86_64" / "initrd.img",
        marker_prefix="CROSVM",
        log_prefix="crosvm-bench",
        output_name="crosvm-capsem-bench-initrd.img",
    )


def block_option(rootfs: Path, engine: str, direct: bool, multiple_workers: bool) -> str:
    return ",".join(
        [
            f"path={rootfs}",
            "ro=true",
            "root=false",
            f"async-executor={engine}",
            f"direct={str(direct).lower()}",
            f"multiple-workers={str(multiple_workers).lower()}",
        ]
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", choices=["epoll", "uring"], default="epoll")
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument("--direct", action="store_true")
    parser.add_argument("--multiple-workers", action="store_true")
    args = parser.parse_args()

    crosvm = ensure_crosvm()
    lane_parts = [args.engine]
    if args.direct:
        lane_parts.append("direct")
    if args.multiple_workers:
        lane_parts.append("workers")
    lane = "_".join(lane_parts)
    work = TARGET / lane
    work.mkdir(parents=True, exist_ok=True)

    kernel = ROOT / "assets" / "x86_64" / "vmlinuz"
    rootfs = ROOT / "assets" / "x86_64" / "rootfs.squashfs"
    initrd = build_crosvm_initrd(work)
    serial_path = work / "serial.log"

    cmd = [
        str(CROSVM),
        "run",
        "--disable-sandbox",
        "--async-executor",
        args.engine,
        "--cpus",
        "num-cores=2",
        "--mem",
        "2048",
        "--serial",
        "type=stdout,hardware=serial,num=1,console,stdin",
        "--block",
        block_option(rootfs, args.engine, args.direct, args.multiple_workers),
        "--initrd",
        str(initrd),
        "--params",
        "console=ttyS0 reboot=k panic=1 random.trust_cpu=1",
        str(kernel),
    ]

    started = time.time()
    proc = subprocess.run(
        cmd,
        cwd=work,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=args.timeout,
    )
    duration = time.time() - started
    serial = proc.stdout + "\n" + proc.stderr
    serial_path.write_text(serial)

    result = {
        "schema": "capsem.crosvm_benchmark.v1",
        "timestamp": time.time(),
        "crosvm": crosvm,
        "engine": args.engine,
        "direct": args.direct,
        "multiple_workers": args.multiple_workers,
        "duration_s": round(duration, 3),
        "returncode": proc.returncode,
        "host": host_metadata(),
        "git_commit": git_commit(),
        "assets": {
            "kernel": str(kernel),
            "rootfs": str(rootfs),
            "initrd_source": str(ROOT / "assets" / "x86_64" / "initrd.img"),
        },
    }
    if proc.returncode == 0:
        result["rootfs"] = extract_json(serial, "CROSVM", "ROOTFS")["rootfs"]
        result["startup"] = extract_json(serial, "CROSVM", "STARTUP")["startup"]
    else:
        result["error"] = "crosvm exited non-zero"

    out = work / "result.json"
    out.write_text(json.dumps(result, indent=2))
    artifact_dir = ROOT / "benchmarks" / "crosvm"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    artifact = artifact_dir / f"data_{project_version()}_{platform.machine()}_{lane}.json"
    artifact.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return proc.returncode


if __name__ == "__main__":
    sys.exit(main())

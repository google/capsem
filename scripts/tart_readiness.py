#!/usr/bin/env python3
"""Prove the cached Tart base image can clone, boot, and accept SSH."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from macos_tart_glowup import (
    DEFAULT_IMAGE,
    OWNED_VM_PREFIX,
    cleanup_vm,
    require_owned_vm,
    run_checked,
    terminate_runner,
    wait_for_guest_ip,
    wait_for_ssh,
)


ROOT = Path(__file__).resolve().parents[1]


def cached_oci_images() -> set[str]:
    completed = run_checked(
        ["tart", "list", "--source", "oci", "--format", "json"],
        capture_output=True,
    )
    payload = json.loads(completed.stdout)
    if not isinstance(payload, list):
        raise RuntimeError("tart list returned a non-list OCI cache")
    return {
        row["Name"]
        for row in payload
        if isinstance(row, dict) and isinstance(row.get("Name"), str)
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--image",
        default=os.environ.get("CAPSEM_TART_IMAGE", DEFAULT_IMAGE),
    )
    parser.add_argument(
        "--require-cache",
        action="store_true",
        help="fail instead of pulling when the OCI base image is not cached",
    )
    parser.add_argument(
        "--report",
        type=Path,
        default=ROOT / "target" / "tart-readiness" / "report.json",
    )
    args = parser.parse_args()

    if sys.platform != "darwin":
        raise RuntimeError("Tart readiness requires macOS")
    if subprocess.run(["uname", "-m"], capture_output=True, text=True).stdout.strip() != "arm64":
        raise RuntimeError("Tart readiness requires Apple Silicon")

    cached_before = args.image in cached_oci_images()
    if args.require_cache and not cached_before:
        raise RuntimeError(
            f"Tart base image is not cached: {args.image}; rerun bootstrap to fetch it"
        )

    started = time.monotonic()
    vm_name = f"{OWNED_VM_PREFIX}readiness-{os.getpid()}-{int(time.time())}"
    require_owned_vm(vm_name)
    runner: subprocess.Popen[str] | None = None
    log_stream = None
    try:
        run_checked(["tart", "clone", args.image, vm_name], timeout=3600)
        run_checked(
            [
                "tart",
                "set",
                vm_name,
                "--cpu",
                "4",
                "--memory",
                "8192",
                "--disk-size",
                "80",
            ]
        )
        args.report.parent.mkdir(parents=True, exist_ok=True)
        log_stream = (args.report.parent / "tart-run.log").open("w")
        runner = subprocess.Popen(
            [
                "tart",
                "run",
                "--no-graphics",
                "--no-audio",
                "--no-clipboard",
                vm_name,
            ],
            stdout=log_stream,
            stderr=subprocess.STDOUT,
            text=True,
        )
        ip = wait_for_guest_ip(vm_name, runner)
        wait_for_ssh(ip)
        report = {
            "schema": "capsem.tart_readiness.v1",
            "image": args.image,
            "cached_before": cached_before,
            "cloned": True,
            "booted": True,
            "ssh_ready": True,
            "memory_mib": 8192,
            "cpu": 4,
            "disk_gib": 80,
            "elapsed_seconds": round(time.monotonic() - started, 1),
        }
        args.report.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(
            f"Tart readiness passed: image={args.image} "
            f"cached_before={str(cached_before).lower()} report={args.report}"
        )
        return 0
    finally:
        cleanup_vm(vm_name)
        terminate_runner(runner, log_stream)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError, ValueError) as error:
        print(f"Tart readiness failed: {error}", file=sys.stderr)
        raise SystemExit(1)

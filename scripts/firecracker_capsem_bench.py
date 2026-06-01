#!/usr/bin/env python3
"""Run Capsem rootfs/startup benchmarks under official Firecracker."""

from __future__ import annotations

import argparse
import json
import platform
import shutil
import sys
import time
from pathlib import Path
from urllib.request import urlopen

from hypervisor_bench_common import (
    ROOT,
    build_initrd as build_benchmark_initrd,
    extract_json as extract_benchmark_json,
    git_commit,
    host_metadata,
    project_version,
    run,
)

TARGET = ROOT / "target" / "firecracker-bench"
BIN_DIR = ROOT / "target" / "firecracker-bin"
FIRECRACKER = BIN_DIR / "firecracker"
RELEASE_URL = "https://github.com/firecracker-microvm/firecracker/releases"


def latest_release() -> str:
    with urlopen(f"{RELEASE_URL}/latest", timeout=30) as response:
        return response.url.rstrip("/").rsplit("/", 1)[-1]


def ensure_firecracker() -> str:
    BIN_DIR.mkdir(parents=True, exist_ok=True)
    if FIRECRACKER.exists():
        version = run([str(FIRECRACKER), "--version"]).stdout.strip().splitlines()[0]
        return version

    version = latest_release()
    arch = platform.machine()
    tgz = BIN_DIR / f"firecracker-{version}-{arch}.tgz"
    url = f"{RELEASE_URL}/download/{version}/firecracker-{version}-{arch}.tgz"
    run(["curl", "-fL", url, "-o", str(tgz)], timeout=120)
    run(["tar", "-xzf", str(tgz), "-C", str(BIN_DIR)], timeout=60)
    src = BIN_DIR / f"release-{version}-{arch}" / f"firecracker-{version}-{arch}"
    shutil.copy2(src, FIRECRACKER)
    FIRECRACKER.chmod(0o755)
    return run([str(FIRECRACKER), "--version"]).stdout.strip().splitlines()[0]


def build_initrd(work: Path, source_initrd: Path) -> Path:
    return build_benchmark_initrd(
        work,
        source_initrd,
        marker_prefix="FIRECRACKER",
        log_prefix="fc-bench",
        output_name="firecracker-capsem-bench-initrd.img",
    )


def kernel_for_firecracker(source_kernel: Path) -> Path:
    file_info = run(["file", str(source_kernel)]).stdout
    if "ELF 64-bit" in file_info:
        return source_kernel
    extractors = [
        Path("/usr/src/linux-gcp-headers-7.0.0-1003/scripts/extract-vmlinux"),
        Path("/usr/src/linux-headers-7.0.0-1003/scripts/extract-vmlinux"),
    ]
    extractor = next((path for path in extractors if path.exists()), None)
    if extractor is None:
        candidates = list(Path("/usr/src").glob("*/scripts/extract-vmlinux"))
        extractor = candidates[0] if candidates else None
    if extractor is None:
        raise RuntimeError("Firecracker needs an ELF vmlinux and no extract-vmlinux tool was found")
    out = TARGET / "vmlinux-capsem"
    TARGET.mkdir(parents=True, exist_ok=True)
    with out.open("wb") as handle:
        subprocess.run([str(extractor), str(source_kernel)], stdout=handle, check=True)
    extracted_info = run(["file", str(out)]).stdout
    if "ELF 64-bit" not in extracted_info:
        raise RuntimeError(f"extracted kernel is not an ELF image: {extracted_info.strip()}")
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", choices=["Sync", "Async"], default="Sync")
    parser.add_argument("--timeout", type=int, default=240)
    args = parser.parse_args()

    version = ensure_firecracker()
    work = TARGET / args.engine.lower()
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)

    kernel = kernel_for_firecracker(ROOT / "assets" / "x86_64" / "vmlinuz")
    rootfs = ROOT / "assets" / "x86_64" / "rootfs.squashfs"
    initrd = build_initrd(work, ROOT / "assets" / "x86_64" / "initrd.img")
    log_path = work / "firecracker.log"
    metrics_path = work / "firecracker-metrics.jsonl"

    config = {
        "boot-source": {
            "kernel_image_path": str(kernel),
            "initrd_path": str(initrd),
            "boot_args": "console=ttyS0 reboot=k panic=1 pci=off random.trust_cpu=1",
        },
        "drives": [
            {
                "drive_id": "rootfs",
                "path_on_host": str(rootfs),
                "is_root_device": False,
                "is_read_only": True,
                "cache_type": "Unsafe",
                "io_engine": args.engine,
            }
        ],
        "machine-config": {
            "vcpu_count": 2,
            "mem_size_mib": 2048,
            "smt": False,
            "track_dirty_pages": False,
            "huge_pages": "None",
        },
        "logger": {
            "log_path": str(log_path),
            "level": "Info",
            "show_level": True,
            "show_log_origin": True,
        },
        "metrics": {"metrics_path": str(metrics_path)},
    }
    config_path = work / "config.json"
    config_path.write_text(json.dumps(config, indent=2))

    started = time.time()
    proc = subprocess.run(
        [
            str(FIRECRACKER),
            "--no-api",
            "--no-seccomp",
            "--id",
            f"capsem-bench-{args.engine.lower()}",
            "--config-file",
            str(config_path),
        ],
        cwd=work,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=args.timeout,
    )
    duration = time.time() - started
    serial = proc.stdout + "\n" + proc.stderr
    (work / "serial.log").write_text(serial)

    result = {
        "schema": "capsem.firecracker_benchmark.v1",
        "timestamp": time.time(),
        "firecracker": version,
        "engine": args.engine,
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
        result["rootfs"] = extract_benchmark_json(serial, "FIRECRACKER", "ROOTFS")["rootfs"]
        result["startup"] = extract_benchmark_json(serial, "FIRECRACKER", "STARTUP")["startup"]
    else:
        result["error"] = "firecracker exited non-zero"

    out = work / "result.json"
    out.write_text(json.dumps(result, indent=2))
    artifact_dir = ROOT / "benchmarks" / "firecracker"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    artifact = artifact_dir / (
        f"data_{project_version()}_{platform.machine()}_{args.engine.lower()}.json"
    )
    artifact.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return proc.returncode


if __name__ == "__main__":
    sys.exit(main())

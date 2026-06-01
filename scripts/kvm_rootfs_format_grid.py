#!/usr/bin/env python3
"""Run KVM rootfs-format x virtio-blk-shape benchmark matrices."""

from __future__ import annotations

import argparse
import itertools
import json
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

from hypervisor_bench_common import ROOT, git_commit, host_metadata, project_version
from kvm_block_shape_grid import parse_csv_u16, parse_seg_maxes, shape_env


ARCH = platform.machine().replace("aarch64", "arm64")
ARTIFACT_DIR = ROOT / "benchmarks" / "kvm-rootfs-format-grid"
TARGET = ROOT / "target" / "kvm-rootfs-format-grid"
SOURCE_ASSETS = ROOT / "assets"

FORMAT_PROFILES = {
    "squashfs-zstd": {
        "mount_type": "squashfs",
        "description": "current production SquashFS zstd rootfs",
    },
    "squashfs-uncompressed": {
        "mount_type": "squashfs",
        "description": "SquashFS with compression disabled for data and metadata",
    },
    "erofs": {
        "mount_type": "erofs",
        "description": "EROFS read-only rootfs image with lz4hc compression",
        "compression": "lz4hc",
    },
}

ZSTD_FORMAT_RE = re.compile(r"^squashfs-zstd-l([1-9]|1[0-9]|2[0-2])$")
EROFS_FORMAT_RE = re.compile(r"^erofs-(uncompressed|lz4|lz4hc)$")


def run(
    cmd: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    timeout: int | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=check,
    )


def link_or_copy(src: Path, dst: Path) -> None:
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.unlink(missing_ok=True)
    try:
        os.link(src, dst)
    except OSError:
        shutil.copy2(src, dst)


def source_arch_dir() -> Path:
    arch_dir = SOURCE_ASSETS / ARCH
    if not arch_dir.exists():
        raise RuntimeError(f"missing source asset directory: {arch_dir}")
    return arch_dir


def variant_assets_dir(format_name: str) -> Path:
    return TARGET / "assets" / format_name


def variant_arch_dir(format_name: str) -> Path:
    return variant_assets_dir(format_name) / ARCH


def just_assets_dir(format_name: str) -> str:
    # justfile recipes prefix assets_dir with the repo root in several places,
    # so pass a repo-relative path even though the script works with absolutes.
    return os.path.relpath(variant_assets_dir(format_name), ROOT)


def extracted_rootfs_dir() -> Path:
    return TARGET / "extracted-rootfs"


def copy_common_assets(format_name: str) -> Path:
    src = source_arch_dir()
    dst = variant_arch_dir(format_name)
    dst.mkdir(parents=True, exist_ok=True)
    for name in ("vmlinuz", "initrd.img", "image-inventory.json"):
        link_or_copy(src / name, dst / name)
    return dst


def ensure_extracted_rootfs(*, rebuild: bool) -> Path:
    root = extracted_rootfs_dir()
    if root.exists() and not rebuild:
        return root
    shutil.rmtree(root, ignore_errors=True)
    root.parent.mkdir(parents=True, exist_ok=True)
    run(
        [
            "unsquashfs",
            "-quiet",
            "-no-progress",
            "-d",
            str(root),
            str(source_arch_dir() / "rootfs.squashfs"),
        ],
        timeout=900,
    )
    return root


def materialize_squashfs_zstd(*, rebuild: bool) -> dict:
    del rebuild
    dst = copy_common_assets("squashfs-zstd")
    link_or_copy(source_arch_dir() / "rootfs.squashfs", dst / "rootfs.squashfs")
    return rootfs_image_metadata("squashfs-zstd")


def materialize_squashfs_zstd_level(format_name: str, *, rebuild: bool) -> dict:
    match = ZSTD_FORMAT_RE.match(format_name)
    if not match:
        raise ValueError(f"invalid zstd format name: {format_name}")
    level = int(match.group(1), 10)
    dst = copy_common_assets(format_name)
    out = dst / "rootfs.squashfs"
    if not out.exists() or rebuild:
        root = ensure_extracted_rootfs(rebuild=rebuild)
        tmp = out.with_suffix(".tmp")
        tmp.unlink(missing_ok=True)
        run(
            [
                "mksquashfs",
                str(root),
                str(tmp),
                "-comp",
                "zstd",
                "-Xcompression-level",
                str(level),
                "-b",
                "128K",
                "-noappend",
                "-processors",
                str(os.cpu_count() or 1),
            ],
            timeout=1800,
        )
        tmp.replace(out)
    return rootfs_image_metadata(format_name)


def materialize_squashfs_uncompressed(*, rebuild: bool) -> dict:
    dst = copy_common_assets("squashfs-uncompressed")
    out = dst / "rootfs.squashfs"
    if not out.exists() or rebuild:
        root = ensure_extracted_rootfs(rebuild=rebuild)
        tmp = out.with_suffix(".tmp")
        tmp.unlink(missing_ok=True)
        run(
            [
                "mksquashfs",
                str(root),
                str(tmp),
                "-no-compression",
                "-b",
                "128K",
                "-noappend",
                "-processors",
                str(os.cpu_count() or 1),
            ],
            timeout=1800,
        )
        tmp.replace(out)
    return rootfs_image_metadata("squashfs-uncompressed")


def erofs_compression(format_name: str) -> str | None:
    if format_name == "erofs":
        return "lz4hc"
    match = EROFS_FORMAT_RE.match(format_name)
    if not match:
        raise ValueError(f"invalid EROFS format name: {format_name}")
    compression = match.group(1)
    if compression == "uncompressed":
        return None
    return compression


def materialize_erofs(format_name: str, *, rebuild: bool) -> dict:
    compression = erofs_compression(format_name)
    dst = copy_common_assets(format_name)
    out = dst / "rootfs.squashfs"
    if not out.exists() or rebuild:
        root = ensure_extracted_rootfs(rebuild=rebuild)
        tmp = out.with_suffix(".tmp")
        tmp.unlink(missing_ok=True)
        erofs_args = ["mkfs.erofs"]
        if compression is not None:
            erofs_args.append(f"-z{compression}")
        erofs_args.extend([str(tmp), str(root)])
        mkfs = shutil.which("mkfs.erofs")
        if mkfs:
            run([mkfs, *erofs_args[1:]], timeout=1800)
        else:
            docker_erofs_args = ["mkfs.erofs"]
            if compression is not None:
                docker_erofs_args.append(f"-z{compression}")
            docker_erofs_args.extend(["/assets/rootfs.tmp", "/rootfs"])
            run(
                [
                    "docker",
                    "run",
                    "--rm",
                    "-v",
                    f"{root}:/rootfs:ro",
                    "-v",
                    f"{dst}:/assets",
                    "debian:bookworm-slim",
                    "bash",
                    "-lc",
                    "apt-get -o Acquire::Check-Valid-Until=false "
                    "-o Acquire::Check-Date=false update && "
                    "apt-get install -y erofs-utils && "
                    f"{shlex.join(docker_erofs_args)}",
                ],
                timeout=1800,
            )
        tmp.replace(out)
    return rootfs_image_metadata(format_name)


def rootfs_image_metadata(format_name: str) -> dict:
    profile = format_profile(format_name)
    path = variant_arch_dir(format_name) / "rootfs.squashfs"
    metadata = {
        "format": format_name,
        "description": profile["description"],
        "mount_type": profile["mount_type"],
        "path": str(path),
        "size_bytes": path.stat().st_size,
        "cmdline_append": f"capsem.rootfs={profile['mount_type']}",
    }
    if "compression" in profile:
        metadata["compression"] = profile["compression"]
    if "compression_level" in profile:
        metadata["compression_level"] = profile["compression_level"]
    return metadata


def format_profile(format_name: str) -> dict:
    if format_name in FORMAT_PROFILES:
        return FORMAT_PROFILES[format_name]
    match = ZSTD_FORMAT_RE.match(format_name)
    if match:
        level = int(match.group(1), 10)
        return {
            "mount_type": "squashfs",
            "description": f"generated SquashFS zstd level {level} rootfs",
            "compression": "zstd",
            "compression_level": level,
        }
    match = EROFS_FORMAT_RE.match(format_name)
    if match:
        compression = match.group(1)
        if compression == "uncompressed":
            return {
                "mount_type": "erofs",
                "description": "generated uncompressed EROFS rootfs",
                "compression": "none",
            }
        return {
            "mount_type": "erofs",
            "description": f"generated EROFS {compression} rootfs",
            "compression": compression,
        }
    raise ValueError(f"unknown format: {format_name}")


def materialize_format(format_name: str, *, rebuild: bool) -> dict:
    if format_name == "squashfs-zstd":
        return materialize_squashfs_zstd(rebuild=rebuild)
    if ZSTD_FORMAT_RE.match(format_name):
        return materialize_squashfs_zstd_level(format_name, rebuild=rebuild)
    if format_name == "squashfs-uncompressed":
        return materialize_squashfs_uncompressed(rebuild=rebuild)
    if format_name == "erofs" or EROFS_FORMAT_RE.match(format_name):
        return materialize_erofs(format_name, rebuild=rebuild)
    raise ValueError(f"unknown format: {format_name}")


def extract_json(output: str, name: str) -> dict:
    pattern = re.compile(
        rf"CAPSEM_KVM_ROOTFS_{re.escape(name)}_JSON_BEGIN\s*(\{{.*?\}})\s*"
        rf"CAPSEM_KVM_ROOTFS_{re.escape(name)}_JSON_END",
        re.DOTALL,
    )
    match = pattern.search(output)
    if not match:
        raise RuntimeError(f"missing {name} JSON marker")
    return json.loads(match.group(1))


def extract_sysfs(output: str) -> dict[str, str]:
    pattern = re.compile(
        r"CAPSEM_KVM_ROOTFS_SYSFS_BEGIN\s*(.*?)\s*CAPSEM_KVM_ROOTFS_SYSFS_END",
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
        "echo CAPSEM_KVM_ROOTFS_SYSFS_BEGIN",
        "printf 'rootfs_mount='; findmnt -n -o FSTYPE /mnt/a 2>/dev/null || true",
        "printf 'mq_dirs='; ls /sys/block/vda/mq 2>/dev/null | wc -l",
        "printf 'max_segments='; cat /sys/block/vda/queue/max_segments",
        "printf 'logical_block_size='; cat /sys/block/vda/queue/logical_block_size",
        "printf 'nr_requests='; cat /sys/block/vda/queue/nr_requests",
        "echo CAPSEM_KVM_ROOTFS_SYSFS_END",
        "capsem-bench storage >/dev/null",
        "echo CAPSEM_KVM_ROOTFS_STORAGE_JSON_BEGIN",
        "cat /tmp/capsem-benchmark.json",
        "echo CAPSEM_KVM_ROOTFS_STORAGE_JSON_END",
        "capsem-bench rootfs >/dev/null",
        "echo CAPSEM_KVM_ROOTFS_ROOTFS_JSON_BEGIN",
        "cat /tmp/capsem-benchmark.json",
        "echo CAPSEM_KVM_ROOTFS_ROOTFS_JSON_END",
    ]
    if startup:
        parts.extend(
            [
                "capsem-bench startup >/dev/null",
                "echo CAPSEM_KVM_ROOTFS_STARTUP_JSON_BEGIN",
                "cat /tmp/capsem-benchmark.json",
                "echo CAPSEM_KVM_ROOTFS_STARTUP_JSON_END",
            ]
        )
    return "; ".join(parts)


def run_cell(
    *,
    format_name: str,
    rootfs_image: dict,
    shape: dict[str, int],
    startup: bool,
    timeout: int,
    scope: str,
) -> dict:
    home = TARGET / "homes" / format_name
    env = {
        **os.environ,
        "CAPSEM_HOME": str(home),
        "CAPSEM_RUN_DIR": str(home / "run"),
        "CAPSEM_DEV_KERNEL_CMDLINE_APPEND": rootfs_image["cmdline_append"],
        **shape_env(shape, scope=scope),
    }
    started = time.time()
    proc = run(
        [
            "just",
            "--set",
            "assets_dir",
            just_assets_dir(format_name),
            "exec",
            guest_command(startup=startup),
        ],
        cwd=ROOT,
        env=env,
        timeout=timeout,
        check=False,
    )
    duration = time.time() - started
    combined = proc.stdout + "\n" + proc.stderr
    result: dict = {
        "format": format_name,
        "shape": shape,
        "returncode": proc.returncode,
        "duration_s": round(duration, 3),
        "sysfs": extract_sysfs(combined),
    }
    if proc.returncode == 0:
        result["storage"] = extract_json(combined, "STORAGE")["storage"]
        result["rootfs"] = extract_json(combined, "ROOTFS")["rootfs"]
        if startup:
            result["startup"] = extract_json(combined, "STARTUP")["startup"]
    else:
        result["error_tail"] = combined[-5000:]
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


def parse_formats(raw: str) -> list[str]:
    formats = [part.strip() for part in raw.split(",") if part.strip()]
    if not formats:
        raise argparse.ArgumentTypeError("formats must contain at least one value")
    unknown = []
    for fmt in formats:
        try:
            format_profile(fmt)
        except ValueError:
            unknown.append(fmt)
    if unknown:
        raise argparse.ArgumentTypeError(f"unknown rootfs format(s): {', '.join(unknown)}")
    return formats


def parse_zstd_levels(raw: str) -> list[int]:
    levels: list[int] = []
    for part in raw.split(","):
        part = part.strip()
        if not part:
            continue
        level = int(part, 10)
        if not 1 <= level <= 22:
            raise argparse.ArgumentTypeError(f"zstd level must be between 1 and 22: {level}")
        if level not in levels:
            levels.append(level)
    return levels


def parse_erofs_compressions(raw: str) -> list[str]:
    compressions: list[str] = []
    allowed = {"none", "uncompressed", "lz4", "lz4hc"}
    for part in raw.split(","):
        compression = part.strip()
        if not compression:
            continue
        if compression not in allowed:
            raise argparse.ArgumentTypeError(
                f"EROFS compression must be one of none,lz4,lz4hc: {compression}"
            )
        if compression == "none":
            compression = "uncompressed"
        if compression not in compressions:
            compressions.append(compression)
    return compressions


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--formats",
        default="squashfs-zstd,squashfs-uncompressed,erofs",
        help="comma-separated rootfs formats to test",
    )
    parser.add_argument(
        "--zstd-levels",
        default="",
        help="append generated SquashFS zstd level variants, e.g. 1,3,9,15,22",
    )
    parser.add_argument(
        "--erofs-compressions",
        default="",
        help="append generated EROFS variants: none,lz4,lz4hc",
    )
    parser.add_argument("--queue-counts", default="1,4,8")
    parser.add_argument("--queue-sizes", default="128,256")
    parser.add_argument("--seg-maxes", default="auto,64")
    parser.add_argument("--logical-block-sizes", default="512,4096")
    parser.add_argument("--startup", action="store_true", help="also run capsem-bench startup")
    parser.add_argument(
        "--scope",
        choices=["rootfs", "all"],
        default="rootfs",
        help="apply shape to read-only rootfs only, or to all KVM block devices",
    )
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--rebuild", action="store_true", help="rebuild generated rootfs variants")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    formats = parse_formats(args.formats)
    for level in parse_zstd_levels(args.zstd_levels):
        name = f"squashfs-zstd-l{level}"
        if name not in formats:
            formats.append(name)
    for compression in parse_erofs_compressions(args.erofs_compressions):
        name = f"erofs-{compression}"
        if name not in formats:
            formats.append(name)
    shapes = build_shapes(args)
    if args.dry_run:
        print(json.dumps({"formats": formats, "count": len(formats) * len(shapes), "shapes": shapes}, indent=2))
        return 0

    rootfs_images = []
    for format_name in formats:
        print(f"materializing {format_name}", flush=True)
        rootfs_images.append(materialize_format(format_name, rebuild=args.rebuild))

    artifact = {
        "schema": "capsem.kvm-rootfs-format-grid.v1",
        "timestamp": time.time(),
        "version": project_version(),
        "arch": ARCH,
        "git_commit": git_commit(),
        "host": host_metadata(),
        "startup": args.startup,
        "scope": args.scope,
        "formats": rootfs_images,
        "shapes": shapes,
        "capabilities": {
            "dax": {
                "status": "not_implemented",
                "reason": (
                    "Capsem KVM rootfs is currently virtio-blk backed. "
                    "DAX requires a separate virtiofs-DAX or pmem-style mapping "
                    "path, so this harness records it as a capability audit item "
                    "rather than pretending a block image can exercise DAX."
                ),
            }
        },
        "results": [],
    }

    total = len(rootfs_images) * len(shapes)
    index = 0
    for rootfs_image in rootfs_images:
        for shape in shapes:
            index += 1
            print(f"[{index}/{total}] {rootfs_image['format']} {shape}", flush=True)
            result = run_cell(
                format_name=rootfs_image["format"],
                rootfs_image=rootfs_image,
                shape=shape,
                startup=args.startup,
                timeout=args.timeout,
                scope=args.scope,
            )
            artifact["results"].append(result)
            if result["returncode"] != 0:
                print(f"  failed: returncode={result['returncode']}", file=sys.stderr, flush=True)
                continue
            rootfs = result["rootfs"]
            print(
                "  rootfs: "
                f"seq={rootfs['seq_read']['throughput_mbps']:.1f} MB/s "
                f"rand={rootfs['rand_read_4k']['iops']:.0f} iops "
                f"small_js={rootfs['small_js_read']['ops_per_sec']:.0f}/s "
                f"meta={rootfs['metadata_stat']['stats_per_sec']:.0f}/s",
                flush=True,
            )

    ARTIFACT_DIR.mkdir(parents=True, exist_ok=True)
    out = ARTIFACT_DIR / f"data_{project_version()}_{ARCH}_{int(time.time())}.json"
    out.write_text(json.dumps(artifact, indent=2) + "\n")
    print(f"wrote {out}")
    return 0 if all(r["returncode"] == 0 for r in artifact["results"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())

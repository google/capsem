#!/usr/bin/env python3
"""Run Capsem rootfs/startup benchmarks under official Firecracker."""

from __future__ import annotations

import argparse
import json
import platform
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from urllib.request import urlopen


ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "target" / "firecracker-bench"
BIN_DIR = ROOT / "target" / "firecracker-bin"
FIRECRACKER = BIN_DIR / "firecracker"
RELEASE_URL = "https://github.com/firecracker-microvm/firecracker/releases"


INIT_SCRIPT = r"""#!/bin/sh
set -eu

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts
mount -t devpts devpts /dev/pts

if [ -e /dev/ttyS0 ]; then
    exec 0</dev/ttyS0 1>/dev/ttyS0 2>/dev/ttyS0
fi

echo "[fc-bench] init start"
ulimit -n 65536
for dev in /sys/block/vd*; do
    if [ -d "$dev" ]; then
        echo none > "$dev/queue/scheduler" 2>/dev/null || true
        echo 0 > "$dev/queue/rotational" 2>/dev/null || true
        echo 4096 > "$dev/queue/read_ahead_kb" 2>/dev/null || true
        echo 256 > "$dev/queue/nr_requests" 2>/dev/null || true
    fi
done

mkdir -p /mnt/a /mnt/b /newroot
echo "[fc-bench] block devices"
ls -la /dev/vd* 2>/dev/null || true
mount -t squashfs /dev/vda /mnt/a
mount -t tmpfs tmpfs /mnt/b
mkdir -p /mnt/b/upper /mnt/b/work
mount -t overlay overlay \
    -o lowerdir=/mnt/a,upperdir=/mnt/b/upper,workdir=/mnt/b/work,redirect_dir=on,metacopy=on \
    /newroot || \
mount -t overlay overlay \
    -o lowerdir=/mnt/a,upperdir=/mnt/b/upper,workdir=/mnt/b/work \
    /newroot

mount -t proc proc /newroot/proc
mount -t sysfs sysfs /newroot/sys
mount -t devtmpfs devtmpfs /newroot/dev
mkdir -p /newroot/dev/pts /newroot/root /newroot/run /newroot/tmp /newroot/usr/local/bin /newroot/usr/local/lib
mount -t devpts devpts /newroot/dev/pts
mount -t tmpfs tmpfs /newroot/root

cp /capsem-bench /newroot/usr/local/bin/capsem-bench
chmod 555 /newroot/usr/local/bin/capsem-bench
rm -rf /newroot/usr/local/lib/capsem_bench
cp -a /capsem_bench /newroot/usr/local/lib/capsem_bench

cat > /newroot/etc/resolv.conf <<'EOF'
nameserver 127.0.0.1
EOF
cat > /newroot/etc/hosts <<'EOF'
127.0.0.1 localhost
127.0.1.1 capsem
::1 localhost ip6-localhost ip6-loopback
EOF

export PATH=/opt/ai-clis/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export HOME=/root
export PYTHONPATH=/usr/local/lib
export NO_UPDATE_NOTIFIER=1
export NPM_CONFIG_UPDATE_NOTIFIER=false
export NPM_CONFIG_FUND=false
export NPM_CONFIG_AUDIT=false
export PIP_DISABLE_PIP_VERSION_CHECK=1
export NODE_OPTIONS="--max-old-space-size=2048"

echo "[fc-bench] rootfs benchmark start"
chroot /newroot /usr/bin/env \
    PATH="$PATH" HOME="$HOME" PYTHONPATH="$PYTHONPATH" \
    NO_UPDATE_NOTIFIER="$NO_UPDATE_NOTIFIER" \
    NPM_CONFIG_UPDATE_NOTIFIER="$NPM_CONFIG_UPDATE_NOTIFIER" \
    NPM_CONFIG_FUND="$NPM_CONFIG_FUND" \
    NPM_CONFIG_AUDIT="$NPM_CONFIG_AUDIT" \
    PIP_DISABLE_PIP_VERSION_CHECK="$PIP_DISABLE_PIP_VERSION_CHECK" \
    NODE_OPTIONS="$NODE_OPTIONS" \
    capsem-bench rootfs
echo "CAPSEM_FIRECRACKER_ROOTFS_JSON_BEGIN"
cat /newroot/tmp/capsem-benchmark.json
echo "CAPSEM_FIRECRACKER_ROOTFS_JSON_END"

echo "[fc-bench] startup benchmark start"
chroot /newroot /usr/bin/env \
    PATH="$PATH" HOME="$HOME" PYTHONPATH="$PYTHONPATH" \
    NO_UPDATE_NOTIFIER="$NO_UPDATE_NOTIFIER" \
    NPM_CONFIG_UPDATE_NOTIFIER="$NPM_CONFIG_UPDATE_NOTIFIER" \
    NPM_CONFIG_FUND="$NPM_CONFIG_FUND" \
    NPM_CONFIG_AUDIT="$NPM_CONFIG_AUDIT" \
    PIP_DISABLE_PIP_VERSION_CHECK="$PIP_DISABLE_PIP_VERSION_CHECK" \
    NODE_OPTIONS="$NODE_OPTIONS" \
    capsem-bench startup
echo "CAPSEM_FIRECRACKER_STARTUP_JSON_BEGIN"
cat /newroot/tmp/capsem-benchmark.json
echo "CAPSEM_FIRECRACKER_STARTUP_JSON_END"

sync
echo "[fc-bench] complete"
reboot -f
while true; do sleep 1; done
"""


def run(cmd: list[str], *, cwd: Path | None = None, timeout: int | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
        timeout=timeout,
    )


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


def project_version() -> str:
    for line in (ROOT / "Cargo.toml").read_text().splitlines():
        if line.startswith("version = "):
            return line.split('"', 2)[1]
    raise RuntimeError("Cargo.toml does not declare a package version")


def build_initrd(work: Path, source_initrd: Path) -> Path:
    initrd_dir = work / "initrd"
    shutil.rmtree(initrd_dir, ignore_errors=True)
    initrd_dir.mkdir(parents=True)
    extract = f"gzip -dc {source_initrd} | cpio -id --quiet"
    subprocess.run(extract, cwd=initrd_dir, shell=True, check=True)
    init = initrd_dir / "init"
    init.write_text(INIT_SCRIPT)
    init.chmod(0o755)
    out = work / "firecracker-capsem-bench-initrd.img"
    pack = f"find . -print | cpio -o -H newc --quiet | gzip -9 > {out}"
    subprocess.run(pack, cwd=initrd_dir, shell=True, check=True)
    return out


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


def extract_json(serial: str, name: str) -> dict:
    pattern = re.compile(
        rf"CAPSEM_FIRECRACKER_{name}_JSON_BEGIN\s*(\{{.*?\}})\s*CAPSEM_FIRECRACKER_{name}_JSON_END",
        re.DOTALL,
    )
    match = pattern.search(serial)
    if not match:
        raise RuntimeError(f"missing {name} JSON marker in serial log")
    return json.loads(match.group(1))


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
        "host": {
            "machine": platform.machine(),
            "system": platform.system(),
            "release": platform.release(),
            "processor": platform.processor(),
        },
        "assets": {
            "kernel": str(kernel),
            "rootfs": str(rootfs),
            "initrd_source": str(ROOT / "assets" / "x86_64" / "initrd.img"),
        },
    }
    if proc.returncode == 0:
        result["rootfs"] = extract_json(serial, "ROOTFS")["rootfs"]
        result["startup"] = extract_json(serial, "STARTUP")["startup"]
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

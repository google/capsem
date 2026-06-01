#!/usr/bin/env python3
"""Shared helpers for reference VMM rootfs/startup benchmarks."""

from __future__ import annotations

import json
import platform
import re
import shutil
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def benchmark_init_script(*, marker_prefix: str, log_prefix: str) -> str:
    return f"""#!/bin/sh
set -eu

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts
mount -t devpts devpts /dev/pts

if [ -e /dev/ttyS0 ]; then
    exec 0</dev/ttyS0 1>/dev/ttyS0 2>/dev/ttyS0
fi

echo "[{log_prefix}] init start"
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
echo "[{log_prefix}] block devices"
ls -la /dev/vd* 2>/dev/null || true
mount -t squashfs /dev/vda /mnt/a
mount -t tmpfs tmpfs /mnt/b
mkdir -p /mnt/b/upper /mnt/b/work
mount -t overlay overlay \\
    -o lowerdir=/mnt/a,upperdir=/mnt/b/upper,workdir=/mnt/b/work,redirect_dir=on,metacopy=on \\
    /newroot || \\
mount -t overlay overlay \\
    -o lowerdir=/mnt/a,upperdir=/mnt/b/upper,workdir=/mnt/b/work \\
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

echo "[{log_prefix}] rootfs benchmark start"
chroot /newroot /usr/bin/env \\
    PATH="$PATH" HOME="$HOME" PYTHONPATH="$PYTHONPATH" \\
    NO_UPDATE_NOTIFIER="$NO_UPDATE_NOTIFIER" \\
    NPM_CONFIG_UPDATE_NOTIFIER="$NPM_CONFIG_UPDATE_NOTIFIER" \\
    NPM_CONFIG_FUND="$NPM_CONFIG_FUND" \\
    NPM_CONFIG_AUDIT="$NPM_CONFIG_AUDIT" \\
    PIP_DISABLE_PIP_VERSION_CHECK="$PIP_DISABLE_PIP_VERSION_CHECK" \\
    NODE_OPTIONS="$NODE_OPTIONS" \\
    capsem-bench rootfs
echo "CAPSEM_{marker_prefix}_ROOTFS_JSON_BEGIN"
cat /newroot/tmp/capsem-benchmark.json
echo "CAPSEM_{marker_prefix}_ROOTFS_JSON_END"

echo "[{log_prefix}] startup benchmark start"
chroot /newroot /usr/bin/env \\
    PATH="$PATH" HOME="$HOME" PYTHONPATH="$PYTHONPATH" \\
    NO_UPDATE_NOTIFIER="$NO_UPDATE_NOTIFIER" \\
    NPM_CONFIG_UPDATE_NOTIFIER="$NPM_CONFIG_UPDATE_NOTIFIER" \\
    NPM_CONFIG_FUND="$NPM_CONFIG_FUND" \\
    NPM_CONFIG_AUDIT="$NPM_CONFIG_AUDIT" \\
    PIP_DISABLE_PIP_VERSION_CHECK="$PIP_DISABLE_PIP_VERSION_CHECK" \\
    NODE_OPTIONS="$NODE_OPTIONS" \\
    capsem-bench startup
echo "CAPSEM_{marker_prefix}_STARTUP_JSON_BEGIN"
cat /newroot/tmp/capsem-benchmark.json
echo "CAPSEM_{marker_prefix}_STARTUP_JSON_END"

sync
echo "[{log_prefix}] complete"
reboot -f
while true; do sleep 1; done
"""


def run(
    cmd: list[str],
    *,
    cwd: Path | None = None,
    timeout: int | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
        timeout=timeout,
    )


def project_version() -> str:
    for line in (ROOT / "Cargo.toml").read_text().splitlines():
        if line.startswith("version = "):
            return line.split('"', 2)[1]
    raise RuntimeError("Cargo.toml does not declare a package version")


def build_initrd(
    work: Path,
    source_initrd: Path,
    *,
    marker_prefix: str,
    log_prefix: str,
    output_name: str,
) -> Path:
    initrd_dir = work / "initrd"
    shutil.rmtree(initrd_dir, ignore_errors=True)
    initrd_dir.mkdir(parents=True)
    extract = f"gzip -dc {source_initrd} | cpio -id --quiet"
    subprocess.run(extract, cwd=initrd_dir, shell=True, check=True)
    init = initrd_dir / "init"
    init.write_text(benchmark_init_script(marker_prefix=marker_prefix, log_prefix=log_prefix))
    init.chmod(0o755)
    out = work / output_name
    pack = f"find . -print | cpio -o -H newc --quiet | gzip -9 > {out}"
    subprocess.run(pack, cwd=initrd_dir, shell=True, check=True)
    return out


def extract_json(serial: str, marker_prefix: str, name: str) -> dict:
    pattern = re.compile(
        rf"CAPSEM_{re.escape(marker_prefix)}_{name}_JSON_BEGIN\s*(\{{.*?\}})\s*"
        rf"CAPSEM_{re.escape(marker_prefix)}_{name}_JSON_END",
        re.DOTALL,
    )
    match = pattern.search(serial)
    if not match:
        raise RuntimeError(f"missing {name} JSON marker for {marker_prefix} in serial log")
    return json.loads(match.group(1))


def host_metadata() -> dict[str, str]:
    return {
        "machine": platform.machine(),
        "system": platform.system(),
        "release": platform.release(),
        "processor": platform.processor(),
    }


def git_commit(path: Path = ROOT) -> str | None:
    proc = run(["git", "rev-parse", "HEAD"], cwd=path, check=False)
    if proc.returncode != 0:
        return None
    return proc.stdout.strip()


def now_timestamp() -> float:
    return time.time()

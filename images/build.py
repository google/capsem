#!/usr/bin/env python3
"""Build VM boot assets using Podman.

Extracts vmlinuz + initrd from Debian ARM64, builds a populated ext4 rootfs
with developer tools and AI CLIs pre-installed.
Output goes to ../assets/.
"""

import hashlib
import shutil
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
ASSETS_DIR = REPO_ROOT / "assets"

IMAGE_TAG = "capsem-kernel-builder"
ROOTFS_IMAGE_TAG = "capsem-rootfs"
ROOTFS_SIZE = "2G"  # ~1.2GB content (AI CLIs are 625MB alone) + ext4 overhead

# Use podman, fall back to docker
RUNTIME = "podman" if shutil.which("podman") else "docker"


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    print(f"  -> {' '.join(cmd)}")
    return subprocess.run(cmd, check=True, **kwargs)


def build_kernel_image():
    """Build the container image that extracts kernel + initrd."""
    print(f"Building kernel extraction image with {RUNTIME}...")
    run([
        RUNTIME, "build",
        "--platform", "linux/arm64",
        "-t", IMAGE_TAG,
        "-f", str(SCRIPT_DIR / "Dockerfile.kernel"),
        str(SCRIPT_DIR),
    ])


def extract_assets():
    """Extract vmlinuz and initrd from the container image."""
    print("Extracting boot assets...")
    ASSETS_DIR.mkdir(parents=True, exist_ok=True)

    # Create a container (don't run it)
    result = run(
        [RUNTIME, "create", "--platform", "linux/arm64", IMAGE_TAG, "/bin/true"],
        capture_output=True,
        text=True,
    )
    container_id = result.stdout.strip()

    try:
        run([RUNTIME, "cp", f"{container_id}:/vmlinuz", str(ASSETS_DIR / "vmlinuz")])
        run([RUNTIME, "cp", f"{container_id}:/initrd.img", str(ASSETS_DIR / "initrd.img")])
    finally:
        run([RUNTIME, "rm", container_id])

    print(f"  vmlinuz:    {ASSETS_DIR / 'vmlinuz'}")
    print(f"  initrd.img: {ASSETS_DIR / 'initrd.img'}")


def ensure_rust_target(target: str):
    """Ensure a rustup target is installed, installing it if missing."""
    result = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        capture_output=True, text=True, check=True,
    )
    if target not in result.stdout.split():
        print(f"  Installing missing rustup target: {target}")
        run(["rustup", "target", "add", target])


def build_agent():
    """Cross-compile capsem-pty-agent and capsem-net-proxy for aarch64-unknown-linux-musl."""
    target = "aarch64-unknown-linux-musl"
    print(f"Cross-compiling guest binaries for {target}...")
    ensure_rust_target(target)
    run([
        "cargo", "build",
        "--release",
        "--target", target,
        "-p", "capsem-agent",
    ], cwd=str(REPO_ROOT))

    # Copy binaries to images/ so Dockerfile.rootfs can COPY them.
    import shutil as _shutil
    release_dir = REPO_ROOT / "target" / "aarch64-unknown-linux-musl" / "release"
    for binary_name in ["capsem-pty-agent", "capsem-net-proxy"]:
        src = release_dir / binary_name
        dst = SCRIPT_DIR / binary_name
        _shutil.copy2(str(src), str(dst))
        print(f"  {binary_name}: {dst} ({dst.stat().st_size} bytes)")


def create_rootfs():
    """Build populated ext4 rootfs with dev tools and AI CLIs."""
    print("Building rootfs container image...")

    # Copy CA cert into images/ so Dockerfile.rootfs can COPY it
    ca_src = REPO_ROOT / "config" / "capsem-ca.crt"
    ca_dst = SCRIPT_DIR / "capsem-ca.crt"
    shutil.copy2(str(ca_src), str(ca_dst))
    print(f"  capsem-ca.crt: {ca_dst}")

    # 1. Build rootfs container (arm64 binaries)
    run([
        RUNTIME, "build",
        "--platform", "linux/arm64",
        "-t", ROOTFS_IMAGE_TAG,
        "-f", str(SCRIPT_DIR / "Dockerfile.rootfs"),
        str(SCRIPT_DIR),
    ])

    # 2. Export container filesystem as tar
    print("Exporting rootfs filesystem...")
    result = run(
        [RUNTIME, "create", "--platform", "linux/arm64",
         ROOTFS_IMAGE_TAG, "/bin/true"],
        capture_output=True, text=True,
    )
    cid = result.stdout.strip()
    tar_path = ASSETS_DIR / "rootfs.tar"
    try:
        run([RUNTIME, "export", cid, "-o", str(tar_path)])
    finally:
        run([RUNTIME, "rm", cid])

    # 3. Create read-only ext4 from tar (mke2fs -d, no mount/privileged needed)
    print(f"Creating {ROOTFS_SIZE} ext4 rootfs image...")
    abs_assets = str(ASSETS_DIR.resolve())
    run([
        RUNTIME, "run", "--rm",
        "-v", f"{abs_assets}:/assets",
        "debian:bookworm-slim", "bash", "-c",
        "apt-get update && apt-get install -y e2fsprogs && "
        "mkdir /rootfs && tar xf /assets/rootfs.tar -C /rootfs && "
        f"mke2fs -t ext4 -d /rootfs -L capsem /assets/rootfs.img {ROOTFS_SIZE}",
    ])

    # 4. Cleanup tar
    tar_path.unlink()

    # 5. Defragment: Docker volume writes through VirtioFS cause severe APFS
    # fragmentation (e.g. 12GB on disk for a 2GB file). A simple copy produces
    # a contiguous file at the correct size.
    img_path = ASSETS_DIR / "rootfs.img"
    tmp_path = ASSETS_DIR / "rootfs.img.tmp"
    print("Defragmenting rootfs.img (APFS volume-mount workaround)...")
    shutil.copy2(str(img_path), str(tmp_path))
    tmp_path.rename(img_path)
    print(f"  rootfs.img: {img_path} ({img_path.stat().st_size // (1024*1024)} MB)")


def generate_checksums():
    print("Generating BLAKE3 checksums...")
    files = [f for f in ["vmlinuz", "initrd.img", "rootfs.img"]
             if (ASSETS_DIR / f).exists()]
    result = subprocess.run(
        ["b3sum"] + files,
        cwd=ASSETS_DIR,
        capture_output=True, text=True, check=True,
    )
    with open(ASSETS_DIR / "B3SUMS", "w") as f:
        f.write(result.stdout)
    for line in result.stdout.strip().split("\n"):
        print(f"  {line}")
    print(f"  B3SUMS: {ASSETS_DIR / 'B3SUMS'}")


def main():
    print(f"Using container runtime: {RUNTIME}")
    build_kernel_image()
    extract_assets()
    build_agent()
    create_rootfs()
    generate_checksums()
    print(f"\nDone! Assets are in {ASSETS_DIR}/")


if __name__ == "__main__":
    main()

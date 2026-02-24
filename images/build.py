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
ROOTFS_SIZE = "2G"

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
    """Cross-compile capsem-pty-agent for aarch64-unknown-linux-musl."""
    target = "aarch64-unknown-linux-musl"
    print(f"Cross-compiling capsem-pty-agent for {target}...")
    ensure_rust_target(target)
    run([
        "cargo", "build",
        "--release",
        "--target", target,
        "-p", "capsem-agent",
    ], cwd=str(REPO_ROOT))

    # Copy the binary to images/ so Dockerfile.rootfs can COPY it.
    src = REPO_ROOT / "target" / "aarch64-unknown-linux-musl" / "release" / "capsem-pty-agent"
    dst = SCRIPT_DIR / "capsem-pty-agent"
    import shutil as _shutil
    _shutil.copy2(str(src), str(dst))
    print(f"  capsem-pty-agent: {dst} ({dst.stat().st_size} bytes)")


def create_rootfs():
    """Build populated ext4 rootfs with dev tools and AI CLIs."""
    print("Building rootfs container image...")

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
    print(f"  rootfs.img: {ASSETS_DIR / 'rootfs.img'}")


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

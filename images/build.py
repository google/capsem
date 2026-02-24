#!/usr/bin/env python3
"""Build VM boot assets using Podman.

Extracts vmlinuz + initrd from Debian ARM64, creates a minimal ext4 rootfs.
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
ROOTFS_SIZE_MB = 64

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


def create_rootfs():
    """Create a minimal empty ext4 rootfs image."""
    rootfs_path = ASSETS_DIR / "rootfs.img"
    print(f"Creating {ROOTFS_SIZE_MB}MB ext4 rootfs...")

    run([
        "dd", "if=/dev/zero", f"of={rootfs_path}",
        "bs=1m", f"count={ROOTFS_SIZE_MB}",
    ])

    # macOS doesn't have mkfs.ext4 natively - use podman to format
    abs_rootfs = str(rootfs_path.resolve())
    run([
        RUNTIME, "run", "--rm",
        "-v", f"{abs_rootfs}:/rootfs.img",
        "debian:bookworm-slim",
        "mkfs.ext4", "-F", "/rootfs.img",
    ])

    print(f"  rootfs.img: {rootfs_path}")


def generate_checksums():
    print("Generating SHA-256 checksums...")
    checksums = []
    for filename in ["vmlinuz", "initrd.img", "rootfs.img"]:
        filepath = ASSETS_DIR / filename
        if filepath.exists():
            sha256 = hashlib.sha256()
            with open(filepath, "rb") as f:
                for chunk in iter(lambda: f.read(4096), b""):
                    sha256.update(chunk)
            hash_hex = sha256.hexdigest()
            checksums.append(f"{hash_hex}  {filename}")
            print(f"  {filename}: {hash_hex}")

    with open(ASSETS_DIR / "SHA256SUMS", "w") as f:
        f.write("\n".join(checksums) + "\n")
    print(f"  SHA256SUMS: {ASSETS_DIR / 'SHA256SUMS'}")


def main():
    print(f"Using container runtime: {RUNTIME}")
    build_kernel_image()
    extract_assets()
    create_rootfs()
    generate_checksums()
    print(f"\nDone! Assets are in {ASSETS_DIR}/")


if __name__ == "__main__":
    main()

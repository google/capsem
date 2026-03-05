#!/usr/bin/env python3
"""Build VM boot assets using Podman/Docker.

Extracts vmlinuz + initrd from Debian ARM64, builds a squashfs rootfs
(zstd-compressed) with developer tools and AI CLIs pre-installed.
Output goes to ../assets/.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
ASSETS_DIR = REPO_ROOT / "assets"

IMAGE_TAG = "capsem-kernel-builder"
ROOTFS_IMAGE_TAG = "capsem-rootfs"

# Use podman, fall back to docker
RUNTIME = "podman" if shutil.which("podman") else "docker"

# In GitHub Actions with docker, use buildx + GHA cache for faster rebuilds.
CI = bool(os.environ.get("GITHUB_ACTIONS"))


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    print(f"  -> {' '.join(cmd)}")
    return subprocess.run(cmd, check=True, **kwargs)


def _docker_build(tag: str, dockerfile: str, context: str):
    """Build a container image, using BuildKit GHA cache in CI."""
    if CI and RUNTIME == "docker":
        run([
            "docker", "buildx", "build",
            "--platform", "linux/arm64",
            "--cache-from", "type=gha,scope=" + tag,
            "--cache-to", "type=gha,mode=max,scope=" + tag,
            "--load",
            "-t", tag,
            "-f", dockerfile,
            context,
        ])
    else:
        run([
            RUNTIME, "build",
            "--platform", "linux/arm64",
            "-t", tag,
            "-f", dockerfile,
            context,
        ])


def build_kernel_image():
    """Build the container image that extracts kernel + initrd."""
    print(f"Building kernel extraction image with {RUNTIME}...")
    _docker_build(IMAGE_TAG, str(SCRIPT_DIR / "Dockerfile.kernel"), str(SCRIPT_DIR))


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
    release_dir = REPO_ROOT / "target" / "aarch64-unknown-linux-musl" / "release"
    for binary_name in ["capsem-pty-agent", "capsem-net-proxy"]:
        src = release_dir / binary_name
        dst = SCRIPT_DIR / binary_name
        shutil.copy2(str(src), str(dst))
        print(f"  {binary_name}: {dst} ({dst.stat().st_size} bytes)")


def create_rootfs():
    """Build squashfs rootfs (zstd-compressed) with dev tools and AI CLIs."""
    print("Building rootfs container image...")

    # Copy CA cert into images/ so Dockerfile.rootfs can COPY it
    ca_src = REPO_ROOT / "config" / "capsem-ca.crt"
    ca_dst = SCRIPT_DIR / "capsem-ca.crt"
    shutil.copy2(str(ca_src), str(ca_dst))
    print(f"  capsem-ca.crt: {ca_dst}")

    # 1. Build rootfs container (arm64 binaries)
    _docker_build(ROOTFS_IMAGE_TAG, str(SCRIPT_DIR / "Dockerfile.rootfs"), str(SCRIPT_DIR))

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

    # 3. Create squashfs image from tar (zstd level 15 for good compression)
    print("Creating squashfs rootfs image (zstd compression)...")
    abs_assets = str(ASSETS_DIR.resolve())
    run([
        RUNTIME, "run", "--rm",
        "-v", f"{abs_assets}:/assets",
        "debian:bookworm-slim", "bash", "-c",
        "apt-get update && apt-get install -y squashfs-tools zstd && "
        "mkdir /rootfs && tar xf /assets/rootfs.tar -C /rootfs && "
        "mksquashfs /rootfs /assets/rootfs.squashfs -comp zstd -Xcompression-level 15 -b 64K -noappend",
    ])

    # 4. Cleanup tar
    tar_path.unlink()

    img_path = ASSETS_DIR / "rootfs.squashfs"
    print(f"  rootfs.squashfs: {img_path} ({img_path.stat().st_size // (1024*1024)} MB)")


def generate_checksums():
    print("Generating BLAKE3 checksums...")
    files = [f for f in ["vmlinuz", "initrd.img", "rootfs.squashfs"]
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
    if CI:
        print("  CI mode: Docker BuildKit GHA cache enabled")
    build_kernel_image()
    extract_assets()
    build_agent()
    create_rootfs()
    generate_checksums()
    print(f"\nDone! Assets are in {ASSETS_DIR}/")


if __name__ == "__main__":
    main()

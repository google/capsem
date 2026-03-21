#!/usr/bin/env python3
"""Build VM boot assets using Podman/Docker.

Extracts vmlinuz + initrd from Debian ARM64, builds a squashfs rootfs
(zstd-compressed) with developer tools and AI CLIs pre-installed.
Output goes to ../assets/.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    import tomli as tomllib  # type: ignore[no-redef]

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
ASSETS_DIR = REPO_ROOT / "assets"

IMAGE_TAG = "capsem-kernel-builder"
ROOTFS_IMAGE_TAG = "capsem-rootfs"

# Use podman, fall back to docker
RUNTIME = "podman" if shutil.which("podman") else "docker"

# In GitHub Actions with docker, use buildx + GHA cache for faster rebuilds.
CI = bool(os.environ.get("GITHUB_ACTIONS"))

FALLBACK_KERNEL_VERSION = "6.6.127"


def resolve_kernel_version(branch: str = "6.6") -> str:
    """Fetch the latest stable kernel version for a given LTS branch.

    Queries https://www.kernel.org/releases.json and returns the latest
    version string matching the branch (e.g. "6.6.129"). Falls back to
    FALLBACK_KERNEL_VERSION on network failure.
    """
    try:
        req = urllib.request.Request(
            "https://www.kernel.org/releases.json",
            headers={"User-Agent": "capsem-build/1.0"},
        )
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode())
    except Exception as e:
        print(f"  Warning: failed to fetch kernel.org releases: {e}")
        print(f"  Falling back to hardcoded {FALLBACK_KERNEL_VERSION}")
        return FALLBACK_KERNEL_VERSION

    prefix = branch + "."
    candidates = []
    for release in data.get("releases", []):
        version = release.get("version", "")
        moniker = release.get("moniker", "")
        iseol = release.get("iseol", False)
        if moniker == "longterm" and version.startswith(prefix) and not iseol:
            # Validate format: X.Y.Z
            if re.fullmatch(r"\d+\.\d+\.\d+", version):
                candidates.append(version)

    if not candidates:
        print(f"  Warning: no {branch}.x LTS versions found in releases.json")
        print(f"  Falling back to hardcoded {FALLBACK_KERNEL_VERSION}")
        return FALLBACK_KERNEL_VERSION

    # Sort by patch version numerically and return the latest
    candidates.sort(key=lambda v: int(v.split(".")[-1]))
    return candidates[-1]


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    print(f"  -> {' '.join(cmd)}")
    return subprocess.run(cmd, check=True, **kwargs)


def _docker_build(tag: str, dockerfile: str, context: str,
                  build_args: dict[str, str] | None = None):
    """Build a container image, using BuildKit GHA cache in CI."""
    args_flags = []
    for k, v in (build_args or {}).items():
        args_flags.extend(["--build-arg", f"{k}={v}"])

    if CI and RUNTIME == "docker":
        run([
            "docker", "buildx", "build",
            "--platform", "linux/arm64",
            "--cache-from", "type=gha,scope=" + tag,
            "--cache-to", "type=gha,mode=max,scope=" + tag,
            "--load",
            *args_flags,
            "-t", tag,
            "-f", dockerfile,
            context,
        ])
    else:
        run([
            RUNTIME, "build",
            "--platform", "linux/arm64",
            *args_flags,
            "-t", tag,
            "-f", dockerfile,
            context,
        ])


def build_kernel_image(kernel_version: str):
    """Build the container image that extracts kernel + initrd."""
    print(f"Building kernel extraction image with {RUNTIME}...")
    _docker_build(
        IMAGE_TAG,
        str(SCRIPT_DIR / "Dockerfile.kernel"),
        str(SCRIPT_DIR),
        build_args={"KERNEL_VERSION": kernel_version},
    )


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


def get_guest_binaries() -> list[str]:
    """Read [[bin]] names from crates/capsem-agent/Cargo.toml (source of truth)."""
    cargo_toml = REPO_ROOT / "crates" / "capsem-agent" / "Cargo.toml"
    with open(cargo_toml, "rb") as f:
        data = tomllib.load(f)
    bins = data.get("bin", [])
    names = [b["name"] for b in bins if "name" in b]
    if not names:
        raise RuntimeError(f"No [[bin]] entries found in {cargo_toml}")
    return names


def build_agent():
    """Cross-compile all guest binaries from capsem-agent for aarch64-unknown-linux-musl."""
    target = "aarch64-unknown-linux-musl"
    binaries = get_guest_binaries()
    print(f"Cross-compiling {len(binaries)} guest binaries for {target}...")
    ensure_rust_target(target)
    run([
        "cargo", "build",
        "--release",
        "--target", target,
        "-p", "capsem-agent",
    ], cwd=str(REPO_ROOT))

    # Copy binaries to images/ so Dockerfile.rootfs can COPY them.
    release_dir = REPO_ROOT / "target" / "aarch64-unknown-linux-musl" / "release"
    for binary_name in binaries:
        src = release_dir / binary_name
        if not src.exists():
            raise RuntimeError(f"Expected binary not found: {src}")
        dst = SCRIPT_DIR / binary_name
        shutil.copy2(str(src), str(dst))
        if dst.stat().st_size == 0:
            raise RuntimeError(f"Binary is empty: {dst}")
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

    # 4. Cleanup tar + legacy rootfs format
    tar_path.unlink()
    legacy_img = ASSETS_DIR / "rootfs.img"
    if legacy_img.exists():
        legacy_img.unlink()
        print("  Removed legacy rootfs.img")

    img_path = ASSETS_DIR / "rootfs.squashfs"
    print(f"  rootfs.squashfs: {img_path} ({img_path.stat().st_size // (1024*1024)} MB)")


def get_cargo_version() -> str:
    """Read workspace version from root Cargo.toml."""
    cargo_toml = REPO_ROOT / "Cargo.toml"
    for line in cargo_toml.read_text().splitlines():
        line = line.strip()
        if line.startswith("version") and "=" in line:
            # version = "0.8.8"
            return line.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("Could not find version in Cargo.toml")


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

    # Generate manifest.json (multi-version rolling manifest).
    version = get_cargo_version()
    assets = []
    for line in result.stdout.strip().split("\n"):
        parts = line.split(None, 1)
        if len(parts) == 2:
            b3hash, filename = parts[0], parts[1].strip()
            filepath = ASSETS_DIR / filename
            size = filepath.stat().st_size if filepath.exists() else 0
            assets.append({"filename": filename, "hash": b3hash, "size": size})

    manifest = {
        "latest": version,
        "releases": {
            version: {"assets": assets},
        },
    }
    manifest_path = ASSETS_DIR / "manifest.json"
    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=2)
    print(f"  manifest.json: {manifest_path} (version {version}, {len(assets)} assets)")


def main():
    parser = argparse.ArgumentParser(
        description="Build Capsem VM boot assets (kernel, initrd, rootfs).",
    )
    parser.add_argument(
        "--kernel-branch", default="6.6",
        help="Target LTS branch (default: 6.6)",
    )
    parser.add_argument(
        "--kernel-version", default=None,
        help="Explicit kernel version (skips auto-detection)",
    )
    parser.add_argument(
        "--arch", default="arm64", choices=["arm64"],
        help="Target architecture (default: arm64)",
    )
    args = parser.parse_args()

    print(f"Using container runtime: {RUNTIME}")
    if CI:
        print("  CI mode: Docker BuildKit GHA cache enabled")

    # Resolve kernel version
    if args.kernel_version:
        kernel_version = args.kernel_version
        print(f"Kernel: {kernel_version} (explicit override)")
    else:
        kernel_version = resolve_kernel_version(args.kernel_branch)
        print(f"Kernel: {kernel_version} (auto-detected from kernel.org {args.kernel_branch}.x LTS)")

    build_kernel_image(kernel_version)
    extract_assets()
    build_agent()
    create_rootfs()
    generate_checksums()
    print(f"\nDone! Assets are in {ASSETS_DIR}/")


if __name__ == "__main__":
    main()

"""Dockerfile generation and build execution from GuestImageConfig.

Renders Dockerfiles via Jinja2 templates and executes Docker/Podman builds
to produce VM boot assets. Supports multi-architecture output (arm64, x86_64).
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import urllib.request
from pathlib import Path
from typing import Any

from jinja2 import Environment, FileSystemLoader

from capsem.builder.doctor import check_container_runtime
from capsem.builder.models import GuestImageConfig, PackageManager

TEMPLATES_DIR = Path(__file__).parent / "templates"
FALLBACK_KERNEL_VERSION = "6.6.127"

# Guest binaries COPY'd into the rootfs (cross-compiled Rust binaries).
GUEST_BINARIES = [
    "capsem-pty-agent",
    "capsem-net-proxy",
    "capsem-mcp-server",
]


def _rootfs_context(config: GuestImageConfig, arch_name: str) -> dict[str, Any]:
    """Build Jinja context for Dockerfile.rootfs.j2."""
    arch = config.build.architectures[arch_name]

    apt_packages: list[str] = []
    if "apt" in config.package_sets:
        apt_packages = list(config.package_sets["apt"].packages)

    python_packages: list[str] = []
    python_install_cmd = "uv pip install --system --break-system-packages"
    if "python" in config.package_sets:
        python_packages = list(config.package_sets["python"].packages)
        python_install_cmd = config.package_sets["python"].install_cmd

    npm_packages: list[str] = []
    npm_prefix = "/opt/ai-clis"
    for provider in config.ai_providers.values():
        if provider.enabled and provider.install:
            if provider.install.manager == PackageManager.NPM:
                npm_packages.extend(provider.install.packages)
                if provider.install.prefix:
                    npm_prefix = provider.install.prefix

    return {
        "arch": arch,
        "arch_name": arch_name,
        "apt_packages": apt_packages,
        "python_packages": python_packages,
        "python_install_cmd": python_install_cmd,
        "npm_packages": npm_packages,
        "npm_prefix": npm_prefix,
        "guest_binaries": GUEST_BINARIES,
    }


def _kernel_context(
    config: GuestImageConfig, arch_name: str, kernel_version: str
) -> dict[str, Any]:
    """Build Jinja context for Dockerfile.kernel.j2."""
    arch = config.build.architectures[arch_name]
    return {
        "arch": arch,
        "arch_name": arch_name,
        "kernel_version": kernel_version,
    }


def generate_build_context(
    template_name: str,
    config: GuestImageConfig,
    arch_name: str,
    **kwargs: Any,
) -> dict[str, Any]:
    """Generate the Jinja template context dict for a given template.

    Args:
        template_name: Template filename (e.g., "Dockerfile.rootfs.j2").
        config: Guest image configuration.
        arch_name: Architecture name (e.g., "arm64", "x86_64").
        **kwargs: Extra context (e.g., kernel_version for kernel template).

    Returns:
        Context dict ready for Jinja rendering.

    Raises:
        ValueError: If template_name is not recognized.
        KeyError: If arch_name is not in config.build.architectures.
    """
    if template_name == "Dockerfile.rootfs.j2":
        ctx = _rootfs_context(config, arch_name)
    elif template_name == "Dockerfile.kernel.j2":
        kernel_version = kwargs.get("kernel_version", "6.6.127")
        ctx = _kernel_context(config, arch_name, kernel_version)
    else:
        raise ValueError(f"Unknown template: {template_name}")

    ctx.update(kwargs)
    return ctx


def render_dockerfile(
    template_name: str,
    config: GuestImageConfig,
    arch_name: str,
    **kwargs: Any,
) -> str:
    """Render a Dockerfile from a Jinja2 template with config context.

    Args:
        template_name: Template filename (e.g., "Dockerfile.rootfs.j2").
        config: Guest image configuration.
        arch_name: Architecture name (e.g., "arm64", "x86_64").
        **kwargs: Extra context (e.g., kernel_version for kernel template).

    Returns:
        Rendered Dockerfile as a string.

    Raises:
        ValueError: If template_name is not recognized.
        KeyError: If arch_name is not in config.build.architectures.
    """
    context = generate_build_context(template_name, config, arch_name, **kwargs)
    env = Environment(
        loader=FileSystemLoader(str(TEMPLATES_DIR)),
        keep_trailing_newline=True,
        trim_blocks=True,
        lstrip_blocks=True,
    )
    template = env.get_template(template_name)
    return template.render(**context)


# ---------------------------------------------------------------------------
# Build execution helpers
# ---------------------------------------------------------------------------


def run_cmd(
    cmd: list[str],
    *,
    cwd: str | Path | None = None,
    capture: bool = False,
    echo: bool = True,
) -> subprocess.CompletedProcess[str]:
    """Run a subprocess command. Single mock seam for tests."""
    if echo:
        print(f"  -> {' '.join(str(c) for c in cmd)}")
    kwargs: dict[str, Any] = {"check": True, "text": True}
    if cwd:
        kwargs["cwd"] = str(cwd)
    if capture:
        kwargs["capture_output"] = True
    return subprocess.run(cmd, **kwargs)


def detect_runtime() -> str:
    """Detect container runtime, raising with fix guidance if missing."""
    result = check_container_runtime()
    if not result.passed:
        raise RuntimeError(f"{result.name}: {result.detail}\n  fix: {result.fix}")
    detail = result.detail.lower()
    if "podman" in detail:
        return "podman"
    return "docker"


def is_ci() -> bool:
    """Return True when running in GitHub Actions."""
    return bool(os.environ.get("GITHUB_ACTIONS"))


def resolve_kernel_version(branch: str = "6.6") -> str:
    """Fetch latest stable kernel version for an LTS branch from kernel.org."""
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
            if re.fullmatch(r"\d+\.\d+\.\d+", version):
                candidates.append(version)

    if not candidates:
        print(f"  Warning: no {branch}.x LTS versions found")
        print(f"  Falling back to hardcoded {FALLBACK_KERNEL_VERSION}")
        return FALLBACK_KERNEL_VERSION

    candidates.sort(key=lambda v: int(v.split(".")[-1]))
    return candidates[-1]


def get_project_version(repo_root: Path) -> str:
    """Read workspace version from root Cargo.toml."""
    cargo_toml = repo_root / "Cargo.toml"
    if not cargo_toml.is_file():
        raise RuntimeError(f"Cargo.toml not found at {cargo_toml}")
    for line in cargo_toml.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith("version") and "=" in stripped:
            return stripped.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("Could not find version in Cargo.toml")


# ---------------------------------------------------------------------------
# Docker operations
# ---------------------------------------------------------------------------


def docker_build(
    runtime: str,
    tag: str,
    dockerfile_path: str | Path,
    context_dir: str | Path,
    platform: str,
    build_args: dict[str, str] | None = None,
    ci_cache: bool = False,
) -> None:
    """Build a container image."""
    args_flags: list[str] = []
    for k, v in (build_args or {}).items():
        args_flags.extend(["--build-arg", f"{k}={v}"])

    if ci_cache and runtime == "docker":
        run_cmd([
            "docker", "buildx", "build",
            "--platform", platform,
            "--cache-from", f"type=gha,scope={tag}",
            "--cache-to", f"type=gha,mode=max,scope={tag}",
            "--load",
            *args_flags,
            "-t", tag,
            "-f", str(dockerfile_path),
            str(context_dir),
        ])
    else:
        run_cmd([
            runtime, "build",
            "--platform", platform,
            *args_flags,
            "-t", tag,
            "-f", str(dockerfile_path),
            str(context_dir),
        ])


def extract_kernel_assets(
    runtime: str,
    image_tag: str,
    platform: str,
    output_dir: Path,
) -> tuple[Path, Path]:
    """Extract vmlinuz and initrd.img from a kernel builder image."""
    output_dir.mkdir(parents=True, exist_ok=True)
    result = run_cmd(
        [runtime, "create", "--platform", platform, image_tag, "/bin/true"],
        capture=True,
    )
    cid = result.stdout.strip()
    vmlinuz = output_dir / "vmlinuz"
    initrd = output_dir / "initrd.img"
    try:
        run_cmd([runtime, "cp", f"{cid}:/vmlinuz", str(vmlinuz)])
        run_cmd([runtime, "cp", f"{cid}:/initrd.img", str(initrd)])
    finally:
        run_cmd([runtime, "rm", cid])
    return vmlinuz, initrd


def export_container_fs(
    runtime: str,
    image_tag: str,
    platform: str,
    output_tar: Path,
) -> None:
    """Export container filesystem as a tar archive."""
    result = run_cmd(
        [runtime, "create", "--platform", platform, image_tag, "/bin/true"],
        capture=True,
    )
    cid = result.stdout.strip()
    try:
        run_cmd([runtime, "export", cid, "-o", str(output_tar)])
    finally:
        run_cmd([runtime, "rm", cid])


def create_squashfs(
    runtime: str,
    tar_path: Path,
    output_path: Path,
    compression: str,
    compression_level: int,
    block_size: str = "64K",
) -> None:
    """Create a squashfs image from a tar archive using a container."""
    abs_dir = str(tar_path.parent.resolve())
    tar_name = tar_path.name
    out_name = output_path.name

    # -Xcompression-level is only valid for zstd and xz
    level_flag = ""
    if compression in ("zstd", "xz"):
        level_flag = f" -Xcompression-level {compression_level}"

    run_cmd([
        runtime, "run", "--rm",
        "-v", f"{abs_dir}:/assets",
        "debian:bookworm-slim", "bash", "-c",
        f"apt-get update && apt-get install -y squashfs-tools zstd && "
        f"mkdir /rootfs && tar xf /assets/{tar_name} -C /rootfs && "
        f"mksquashfs /rootfs /assets/{out_name} -comp {compression}{level_flag} -b {block_size} -noappend",
    ])


def cross_compile_agent(
    rust_target: str,
    repo_root: Path,
    output_dir: Path,
) -> list[Path]:
    """Cross-compile guest agent binaries for a given Rust target."""
    # Ensure target installed
    try:
        result = run_cmd(
            ["rustup", "target", "list", "--installed"],
            capture=True, echo=False,
        )
        if rust_target not in result.stdout.split():
            print(f"  Installing missing rustup target: {rust_target}")
            run_cmd(["rustup", "target", "add", rust_target])
    except Exception:
        run_cmd(["rustup", "target", "add", rust_target])

    run_cmd([
        "cargo", "build", "--release",
        "--target", rust_target,
        "-p", "capsem-agent",
    ], cwd=repo_root)

    release_dir = repo_root / "target" / rust_target / "release"
    output_dir.mkdir(parents=True, exist_ok=True)
    copied: list[Path] = []
    for binary in GUEST_BINARIES:
        src = release_dir / binary
        if not src.exists():
            raise RuntimeError(f"Expected binary not found: {src}")
        dst = output_dir / binary
        shutil.copy2(str(src), str(dst))
        if dst.stat().st_size == 0:
            raise RuntimeError(f"Binary is empty: {dst}")
        copied.append(dst)
    return copied


def extract_tool_versions(
    runtime: str,
    image_tag: str,
    platform: str,
    output_dir: Path,
) -> None:
    """Extract tool versions from rootfs image."""
    version_script = (
        "echo \"python=$(python3 --version 2>&1 | awk '{print $2}')\";"
        "echo \"node=$(node --version 2>&1 | tr -d v)\";"
        "echo \"npm=$(npm --version 2>&1)\";"
        "echo \"uv=$(uv --version 2>&1 | awk '{print $2}')\";"
        "echo \"pip=$(pip3 --version 2>&1 | awk '{print $2}')\";"
        "echo \"git=$(git --version 2>&1 | awk '{print $3}')\";"
        "for cli in claude gemini codex; do "
        "  ver=$(/opt/ai-clis/bin/$cli --version 2>/dev/null | head -1 || echo 'N/A'); "
        "  echo \"$cli=$ver\"; "
        "done;"
    )
    result = run_cmd(
        [runtime, "run", "--rm", "--platform", platform,
         image_tag, "bash", "-c", version_script],
        capture=True,
    )
    versions_path = output_dir / "tool-versions.txt"
    versions_path.write_text(result.stdout)


def generate_checksums(output_dir: Path, version: str) -> Path:
    """Generate BLAKE3 checksums and manifest.json for all assets."""
    # Collect all asset files across arch subdirs
    arch_dirs = [d for d in output_dir.iterdir() if d.is_dir()]
    all_files: list[str] = []
    for arch_dir in sorted(arch_dirs):
        arch_name = arch_dir.name
        for f in sorted(arch_dir.iterdir()):
            if f.is_file() and f.name in ("vmlinuz", "initrd.img", "rootfs.squashfs"):
                all_files.append(f"{arch_name}/{f.name}")

    if not all_files:
        # Flat layout fallback
        for f in ("vmlinuz", "initrd.img", "rootfs.squashfs"):
            if (output_dir / f).is_file():
                all_files.append(f)

    result = run_cmd(
        ["b3sum"] + all_files,
        cwd=output_dir, capture=True,
    )
    (output_dir / "B3SUMS").write_text(result.stdout)

    # Build per-arch manifest
    releases: dict[str, Any] = {}
    for line in result.stdout.strip().split("\n"):
        parts = line.split(None, 1)
        if len(parts) != 2:
            continue
        b3hash, filepath = parts[0], parts[1].strip()
        full_path = output_dir / filepath
        size = full_path.stat().st_size if full_path.exists() else 0

        if "/" in filepath:
            arch_name, filename = filepath.split("/", 1)
            arch_assets = releases.setdefault(arch_name, {"assets": []})
            arch_assets["assets"].append({
                "filename": filename, "hash": b3hash, "size": size,
            })
        else:
            flat_assets = releases.setdefault("assets", [])
            flat_assets.append({
                "filename": filepath, "hash": b3hash, "size": size,
            })

    manifest = {"latest": version, "releases": {version: releases}}
    manifest_path = output_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest_path


# ---------------------------------------------------------------------------
# Build context assembly
# ---------------------------------------------------------------------------


def prepare_build_context(
    config: GuestImageConfig,
    arch_name: str,
    template_name: str,
    context_dir: Path,
    repo_root: Path,
    **kwargs: Any,
) -> Path:
    """Write rendered Dockerfile and copy required files into a build context."""
    # Render Dockerfile
    dockerfile_content = render_dockerfile(template_name, config, arch_name, **kwargs)
    dockerfile_path = context_dir / "Dockerfile"
    dockerfile_path.write_text(dockerfile_content)

    if "rootfs" in template_name:
        # CA cert
        shutil.copy2(
            str(repo_root / "config" / "capsem-ca.crt"),
            str(context_dir / "capsem-ca.crt"),
        )
        # Shell config
        for name in ("capsem-bashrc", "banner.txt", "tips.txt"):
            shutil.copy2(
                str(repo_root / "images" / name),
                str(context_dir / name),
            )
        # Diagnostics
        diag_src = repo_root / "images" / "diagnostics"
        diag_dst = context_dir / "diagnostics"
        if diag_src.is_dir():
            shutil.copytree(str(diag_src), str(diag_dst), dirs_exist_ok=True)
        # Doctor + bench scripts
        for name in ("capsem-doctor", "capsem-bench"):
            src = repo_root / "images" / name
            if src.is_file():
                shutil.copy2(str(src), str(context_dir / name))
        # Agent binaries (if they exist in context already from cross_compile_agent)
        # They may have been copied to context_dir by the pipeline before this call

    elif "kernel" in template_name:
        # Defconfig -- preserve directory structure for COPY {{ arch.defconfig }}
        arch = config.build.architectures[arch_name]
        defconfig_src = repo_root / "guest" / "config" / arch.defconfig
        defconfig_dst = context_dir / arch.defconfig
        defconfig_dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(str(defconfig_src), str(defconfig_dst))
        # capsem-init
        shutil.copy2(
            str(repo_root / "images" / "capsem-init"),
            str(context_dir / "capsem-init"),
        )

    return dockerfile_path


# ---------------------------------------------------------------------------
# Pipeline orchestrators
# ---------------------------------------------------------------------------


def build_image(
    config: GuestImageConfig,
    arch_name: str,
    *,
    template: str = "rootfs",
    output_dir: Path | None = None,
    kernel_version: str | None = None,
    repo_root: Path | None = None,
) -> None:
    """Build a Docker image for the given architecture.

    Full pipeline for one arch+template. Outputs to output_dir/{arch_name}/.
    """
    import tempfile

    from capsem.builder.doctor import check_cross_target

    if repo_root is None:
        repo_root = Path.cwd()
    if output_dir is None:
        output_dir = repo_root / "assets"

    arch = config.build.architectures[arch_name]
    runtime = detect_runtime()
    ci = is_ci()

    # Doctor check: cross-compilation target
    if template == "rootfs":
        target_check = check_cross_target(arch.rust_target)
        if not target_check.passed:
            raise RuntimeError(
                f"{target_check.name}: {target_check.detail}\n  fix: {target_check.fix}"
            )

    # Per-arch output directory
    arch_output = output_dir / arch_name
    arch_output.mkdir(parents=True, exist_ok=True)

    template_name = f"Dockerfile.{template}.j2"
    tag = f"capsem-{template}-{arch_name}"

    with tempfile.TemporaryDirectory(prefix="capsem-build-") as tmpdir:
        context_dir = Path(tmpdir)

        if template == "kernel":
            # Resolve kernel version
            if kernel_version is None:
                kernel_version = resolve_kernel_version(arch.kernel_branch)
            print(f"Kernel: {kernel_version}")

            prepare_build_context(
                config, arch_name, template_name, context_dir, repo_root,
                kernel_version=kernel_version,
            )
            docker_build(
                runtime, tag, context_dir / "Dockerfile", context_dir,
                arch.docker_platform,
                build_args={"KERNEL_VERSION": kernel_version},
                ci_cache=ci,
            )
            vmlinuz, initrd = extract_kernel_assets(
                runtime, tag, arch.docker_platform, arch_output,
            )
            print(f"  vmlinuz:    {vmlinuz}")
            print(f"  initrd.img: {initrd}")

        elif template == "rootfs":
            # Cross-compile agent binaries
            print(f"Cross-compiling guest binaries for {arch.rust_target}...")
            binaries = cross_compile_agent(arch.rust_target, repo_root, context_dir)
            for b in binaries:
                print(f"  {b.name}: {b.stat().st_size} bytes")

            prepare_build_context(
                config, arch_name, template_name, context_dir, repo_root,
            )
            docker_build(
                runtime, tag, context_dir / "Dockerfile", context_dir,
                arch.docker_platform, ci_cache=ci,
            )

            # Export and compress
            tar_path = arch_output / "rootfs.tar"
            print("Exporting rootfs filesystem...")
            export_container_fs(runtime, tag, arch.docker_platform, tar_path)

            print(f"Creating squashfs ({config.build.compression.value} compression)...")
            squashfs_path = arch_output / "rootfs.squashfs"
            create_squashfs(
                runtime, tar_path, squashfs_path,
                config.build.compression.value,
                config.build.compression_level,
            )
            tar_path.unlink(missing_ok=True)

            print("Extracting tool versions...")
            extract_tool_versions(runtime, tag, arch.docker_platform, arch_output)

            print(f"  rootfs.squashfs: {squashfs_path}")


def build_all_architectures(
    config: GuestImageConfig,
    *,
    template: str = "rootfs",
    output_dir: Path | None = None,
    kernel_version: str | None = None,
    repo_root: Path | None = None,
) -> None:
    """Build Docker images for all configured architectures."""
    if repo_root is None:
        repo_root = Path.cwd()
    if output_dir is None:
        output_dir = repo_root / "assets"

    for arch_name in config.build.architectures:
        print(f"\n=== Building {template} for {arch_name} ===")
        build_image(
            config, arch_name,
            template=template,
            output_dir=output_dir,
            kernel_version=kernel_version,
            repo_root=repo_root,
        )

    version = get_project_version(repo_root)
    print(f"\nGenerating checksums (version {version})...")
    generate_checksums(output_dir, version)

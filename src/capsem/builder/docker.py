"""Dockerfile generation and build execution from GuestImageConfig.

Renders Dockerfiles via Jinja2 templates and executes Docker/Podman builds
to produce VM boot assets. Supports multi-architecture output (arm64, x86_64).
"""

from __future__ import annotations

import datetime
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path
from typing import Any

from jinja2 import Environment, FileSystemLoader

from capsem.builder.doctor import check_container_runtime
from capsem.builder.models import ErofsConfig, GuestImageConfig

TEMPLATES_DIR = Path(__file__).resolve().parents[3] / "config" / "docker"
FALLBACK_KERNEL_VERSION = "7.0.11"
DEFAULT_EROFS_UTILS_IMAGE = "debian:bookworm-slim"
ZSTD_EROFS_UTILS_IMAGE = "debian:trixie-slim"
BOOT_ASSETS = ("vmlinuz", "initrd.img")
ROOTFS_ASSET_PREFERENCE = ("rootfs.erofs",)
OBOM_ASSET = "obom.cdx.json"
BUILD_LEDGER_NAME = "build-ledger.log"

# Guest binaries COPY'd into the rootfs (cross-compiled Rust binaries).
GUEST_BINARIES = [
    "capsem-pty-agent",
    "capsem-net-proxy",
    "capsem-dns-proxy",
    "capsem-mcp-server",
    "capsem-sysutil",
]

# --- Single source of truth for rootfs artifacts from guest/artifacts/ ---
# Scripts and tools that must be copied into the rootfs build context and
# appear in the rendered Dockerfile.  doctor.py and validate.py import these
# constants so there is exactly ONE list to maintain.

# Individual files -> /usr/local/bin/ (chmod 755)
ROOTFS_SCRIPTS = ["capsem-doctor", "capsem-bench", "snapshots"]

# Directories copied into context (special destinations in Dockerfile)
ROOTFS_SCRIPT_DIRS = ["capsem_bench", "diagnostics"]

# Shell config / text files (not executable scripts)
ROOTFS_SUPPORT_FILES = ["capsem-bashrc", "banner.txt", "tips.txt"]


def enforce_guest_binary_perms(paths: list[Path]) -> None:
    """Apply chmod 555 to guest binaries on the host.

    The container-native build chmods inside the container, but Docker-for-Mac
    bind-mount semantics sometimes let an exec/write bit survive on the host.
    Re-applying on the host guarantees the guest-binary read-only invariant
    (CLAUDE.md) regardless of container runtime quirks.
    """
    for p in paths:
        if not p.exists():
            raise FileNotFoundError(p)
        os.chmod(p, 0o555)

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
    if "npm" in config.package_sets:
        npm_packages.extend(config.package_sets["npm"].packages)
    curl_installs: list[str] = []
    if "curl" in config.package_sets:
        curl_installs.extend(config.package_sets["curl"].packages)

    return {
        "arch": arch,
        "arch_name": arch_name,
        "apt_packages": apt_packages,
        "python_packages": python_packages,
        "python_install_cmd": python_install_cmd,
        "npm_packages": npm_packages,
        "npm_prefix": npm_prefix,
        "curl_installs": curl_installs,
        "guest_binaries": GUEST_BINARIES,
        "profile_root_seed": config.profile_root_seed,
        "profile_build_script": config.profile_build_script,
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
        kernel_version = kwargs.get("kernel_version", FALLBACK_KERNEL_VERSION)
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
    """Validate docker is available, raising with fix guidance if missing."""
    result = check_container_runtime()
    if not result.passed:
        raise RuntimeError(f"{result.name}: {result.detail}\n  fix: {result.fix}")
    return "docker"


def is_ci() -> bool:
    """Return True when running in GitHub Actions."""
    return bool(os.environ.get("GITHUB_ACTIONS"))


# Maximum acceptable clock skew (seconds) between host and container VM.
MAX_CLOCK_SKEW_SECONDS = 30


def sync_container_clock() -> None:
    """Sync container VM clock with host to prevent apt date validation errors.

    On macOS, Colima runs containers inside a Linux VM whose clock can drift
    after host sleep/wake. When the VM clock falls behind, Debian apt-get
    rejects release files as "not valid yet" (exit 100).

    This sets the VM clock to the current host UTC time before builds.
    Silently does nothing on native Linux (no VM layer) or on errors.
    """
    if sys.platform != "darwin":
        return

    now = datetime.datetime.now(datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%SZ"
    )

    try:
        run_cmd(
            ["docker", "run", "--rm", "--privileged",
             "alpine", "date", "-s", now],
            capture=True, echo=False,
        )
    except Exception:
        pass  # Best effort -- apt-get options are the fallback


def resolve_kernel_version(branch: str = "auto") -> str:
    """Fetch the latest kernel version from kernel.org.

    `branch` controls selection:
      - "auto" (default): newest non-EOL longterm (LTS) branch, latest patch.
        Always-fresh; no human bumps required.
      - "X.Y" (e.g. "7.0" or "6.18"): pin to that stable/LTS branch,
        latest patch. Use for reproducibility / security freeze.

    Falls back to `FALLBACK_KERNEL_VERSION` on any network/parse error.
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

    # Collect (major, minor, patch) for every non-EOL stable/LTS release with
    # a strict X.Y.Z version string. Mainline rc releases are deliberately
    # excluded from reproducible guest builds.
    stable_or_lts: list[tuple[int, int, int, str]] = []
    for release in data.get("releases", []):
        version = release.get("version", "")
        moniker = release.get("moniker")
        if moniker not in {"stable", "longterm"} or release.get("iseol"):
            continue
        if not re.fullmatch(r"\d+\.\d+\.\d+", version):
            continue
        a, b, c = (int(x) for x in version.split("."))
        stable_or_lts.append((a, b, c, moniker))

    if not stable_or_lts:
        print("  Warning: no stable/LTS releases found in kernel.org feed")
        print(f"  Falling back to hardcoded {FALLBACK_KERNEL_VERSION}")
        return FALLBACK_KERNEL_VERSION

    if branch == "auto":
        # Highest LTS branch (by major, minor), then highest patch on it.
        lts = [(a, b, c) for (a, b, c, moniker) in stable_or_lts if moniker == "longterm"]
        if not lts:
            print("  Warning: no longterm releases found in kernel.org feed")
            print(f"  Falling back to hardcoded {FALLBACK_KERNEL_VERSION}")
            return FALLBACK_KERNEL_VERSION
        lts.sort()
        a, b, _ = lts[-1]
        patches = sorted(c for (x, y, c) in lts if (x, y) == (a, b))
        version = f"{a}.{b}.{patches[-1]}"
        print(f"  Auto-selected newest LTS: {version}")
        return version

    # Explicit pin: keep only the requested major.minor branch.
    try:
        want_a, want_b = (int(x) for x in branch.split("."))
    except ValueError:
        print(f"  Warning: invalid kernel_branch {branch!r} (want 'auto' or 'X.Y')")
        print(f"  Falling back to hardcoded {FALLBACK_KERNEL_VERSION}")
        return FALLBACK_KERNEL_VERSION
    patches = sorted(c for (a, b, c, _) in stable_or_lts if (a, b) == (want_a, want_b))
    if not patches:
        print(f"  Warning: no non-EOL {branch}.x stable/LTS releases on kernel.org")
        print(f"  Falling back to hardcoded {FALLBACK_KERNEL_VERSION}")
        return FALLBACK_KERNEL_VERSION
    return f"{want_a}.{want_b}.{patches[-1]}"


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


def remove_image(runtime: str, tag: str) -> None:
    """Remove a container image by tag. Silently ignores missing images."""
    try:
        run_cmd([runtime, "rmi", "-f", tag], capture=True)
    except RuntimeError:
        pass


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

    if ci_cache:
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


def create_erofs(
    runtime: str,
    tar_path: Path,
    output_path: Path,
    compression: str,
    cluster_size: str | None = None,
    compression_level: str | None = None,
) -> None:
    """Create an EROFS image from a tar archive using a container."""
    if compression not in {"lz4", "lz4hc", "zstd"}:
        raise ValueError(f"unsupported EROFS compression: {compression}")
    if compression == "zstd" and compression_level is None:
        compression_level = "15"

    if compression_level is not None:
        level = int(compression_level)
        if compression == "lz4":
            raise ValueError("lz4 EROFS compression does not accept a level")
        if compression == "lz4hc" and not 0 <= level <= 12:
            raise ValueError("lz4hc EROFS compression level must be between 0 and 12")
        if compression == "zstd" and not 0 <= level <= 22:
            raise ValueError("zstd EROFS compression level must be between 0 and 22")

    tar_abs = tar_path.resolve()
    output_abs = output_path.resolve()
    common_dir = Path(os.path.commonpath([tar_abs.parent, output_abs.parent]))
    tar_rel = tar_abs.relative_to(common_dir).as_posix()
    out_rel = output_abs.relative_to(common_dir).as_posix()
    out_dir = Path(out_rel).parent.as_posix()
    image = erofs_utils_image_for(compression)
    cluster_flag = f" -C{cluster_size}" if cluster_size else ""
    level_flag = f",level={compression_level}" if compression_level else ""
    mkdir_output = "" if out_dir == "." else f"mkdir -p /assets/{out_dir} && "

    run_cmd([
        runtime, "run", "--rm",
        "-v", f"{common_dir}:/assets",
        image, "bash", "-c",
        f"DEBIAN_FRONTEND=noninteractive apt-get "
        f"-o Acquire::Check-Valid-Until=false -o Acquire::Check-Date=false update && "
        f"DEBIAN_FRONTEND=noninteractive apt-get install -y erofs-utils && "
        f"mkdir /rootfs && {mkdir_output}tar xf /assets/{tar_rel} -C /rootfs && "
        f"mkfs.erofs -Enosbcrc -z{compression}{level_flag}{cluster_flag} "
        f"/assets/{out_rel} /rootfs",
    ])


def erofs_utils_image_for(compression: str) -> str:
    """Return the container image used to create an EROFS image."""
    if compression == "zstd":
        return ZSTD_EROFS_UTILS_IMAGE
    return DEFAULT_EROFS_UTILS_IMAGE


def experimental_erofs_build_config(
    env: dict[str, str] | os._Environ[str] | None = None,
    defaults: ErofsConfig | None = None,
) -> tuple[bool, str, str | None, str | None]:
    """Return EROFS build settings from config defaults and env overrides."""
    source = os.environ if env is None else env
    enabled = defaults.enabled if defaults is not None else False
    if "CAPSEM_BUILD_EXPERIMENTAL_EROFS" in source:
        enabled = source.get("CAPSEM_BUILD_EXPERIMENTAL_EROFS") == "1"
    if not enabled:
        raise ValueError("EROFS build cannot be disabled for the 1.3 asset contract")
    compression = (
        source.get("CAPSEM_BUILD_EROFS_COMPRESSION")
        or (defaults.compression.value if defaults is not None else "lz4hc")
    )
    if compression not in {"lz4", "lz4hc", "zstd"}:
        raise ValueError(
            "CAPSEM_BUILD_EROFS_COMPRESSION must be one of: lz4, lz4hc, zstd"
        )
    cluster_size = source.get("CAPSEM_BUILD_EROFS_CLUSTER_SIZE") or (
        str(defaults.cluster_size) if defaults is not None and defaults.cluster_size else None
    )
    compression_level = source.get("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL") or (
        str(defaults.compression_level)
        if defaults is not None and defaults.compression_level is not None
        else None
    )
    if compression == "zstd" and compression_level is None:
        compression_level = "15"
    if compression_level is not None:
        level = int(compression_level)
        if compression == "lz4":
            raise ValueError("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL is not valid for lz4")
        if compression == "lz4hc" and not 0 <= level <= 12:
            raise ValueError("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL must be 0..12 for lz4hc")
        if compression == "zstd" and not 0 <= level <= 22:
            raise ValueError("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL must be 0..22 for zstd")
    return enabled, compression, cluster_size, compression_level


def container_compile_agent(
    rust_target: str,
    repo_root: Path,
    output_dir: Path,
) -> list[Path]:
    """Compile guest agent binaries inside a Linux container.

    Used on macOS to avoid local cross-linker toolchain issues. Builds natively
    inside a container with per-arch volume caching to prevent cache clobbering
    between arm64 and x86_64 builds.
    """
    runtime = detect_runtime()
    platform = "linux/arm64" if "aarch64" in rust_target else "linux/amd64"
    arch_suffix = "arm64" if "aarch64" in rust_target else "x86_64"
    target_volume = f"capsem-agent-target-{arch_suffix}"
    output_dir.mkdir(parents=True, exist_ok=True)

    # Build all shell commands from GUEST_BINARIES constant
    cp_cmds = " && ".join(
        f"cp target/{rust_target}/release/{b} /output/{b}"
        for b in GUEST_BINARIES
    )
    rm_cmds = " ".join(f"/output/{b}" for b in GUEST_BINARIES)
    chmod_cmds = " ".join(f"/output/{b}" for b in GUEST_BINARIES)
    file_cmds = " && ".join(f"ls -l /output/{b}" for b in GUEST_BINARIES)

    # Pre-pull the image so failures are clear, not buried in a long docker run
    image = "rust:slim-bookworm"
    try:
        run_cmd([runtime, "image", "inspect", image], capture=True, echo=False)
    except subprocess.CalledProcessError:
        print(f"  Pulling {image} ({platform}) ...")
        run_cmd([runtime, "pull", "--platform", platform, image])

    print(f"  Container build ({platform}) ...")
    # Source is mounted :ro to protect the host. We symlink everything into
    # a writable /build dir so cargo can generate Cargo.lock without modifying
    # the host. The target dir and registry are persistent named volumes.
    rustup_volume = f"capsem-rustup-{arch_suffix}"
    run_cmd([
        runtime, "run", "--rm",
        "--platform", platform,
        "-v", f"{repo_root.resolve()}:/src:ro",
        "-v", f"{output_dir.resolve()}:/output",
        "-v", "capsem-cargo-registry:/usr/local/cargo/registry",
        "-v", "capsem-cargo-git:/usr/local/cargo/git",
        "-v", f"{rustup_volume}:/usr/local/rustup",
        "-v", f"{target_volume}:/build/target",
        "-w", "/build",
        image, "bash", "-c",
        f'for f in /src/*; do b=$(basename "$f"); [ "$b" != target ] && [ "$b" != Cargo.lock ] && [ "$b" != crates ] && ln -s "$f" /build/; done && '
        f"cp -r /src/crates /build/crates && "
        f"apt-get update -qq && apt-get install -y -qq musl-tools >/dev/null 2>&1 && "
        f"rustup target add {rust_target} && "
        f"cargo build --release --target {rust_target} -p capsem-agent && "
        f"rm -f {rm_cmds} && "
        f"{cp_cmds} && chmod 555 {chmod_cmds} && {file_cmds}",
    ])

    copied: list[Path] = []
    for binary in GUEST_BINARIES:
        dst = output_dir / binary
        if not dst.exists():
            raise RuntimeError(f"Expected binary not found after container build: {dst}")
        if dst.stat().st_size == 0:
            raise RuntimeError(f"Binary is empty: {dst}")
        copied.append(dst)

    enforce_guest_binary_perms(copied)
    return copied


def cross_compile_agent(
    rust_target: str,
    repo_root: Path,
    output_dir: Path,
) -> list[Path]:
    """Cross-compile guest agent binaries for a given Rust target.

    On macOS, this delegates to container_compile_agent to avoid complex
    local cross-linker setup for x86_64.
    """
    # Use container build on macOS for cross-arch or if specifically requested.
    # For now, let's follow the plan and ensure it uses container on macOS.
    if sys.platform == "darwin":
        print(f"  macOS detected: using container-native build for {rust_target}")
        return container_compile_agent(rust_target, repo_root, output_dir)

    # Native cross-compile (Linux/CI)
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
    enforce_guest_binary_perms(copied)
    return copied


def build_version_script(config: GuestImageConfig) -> str:
    """Build a shell script that extracts tool versions from config.

    Returns a bash script that prints grouped key=value lines to stdout.
    The script is assembled from version_commands in build config and package
    sets. Profile-owned build scripts install agent CLIs; they are not authored
    through builder config.
    """
    lines: list[str] = []

    # -- System: build-level tools (node, npm, uv, pip) + apt packages --
    system_cmds: list[tuple[str, str]] = []
    for key, cmd in config.build.version_commands.items():
        system_cmds.append((key, cmd))
    if "apt" in config.package_sets:
        for key, cmd in config.package_sets["apt"].version_commands.items():
            system_cmds.append((key, cmd))
    if system_cmds:
        lines.append('echo "# System";')
        for key, cmd in system_cmds:
            lines.append(f'echo "{key}=$({cmd} || echo \'N/A\')";')

    # -- Python packages --
    if "python" in config.package_sets:
        py_cmds = config.package_sets["python"].version_commands
        if py_cmds:
            lines.append('echo "# Python";')
            for key, cmd in py_cmds.items():
                lines.append(f'echo "{key}=$({cmd} || echo \'N/A\')";')

    return "\n".join(lines)


def _validate_tool_versions(
    content: str, config: GuestImageConfig,
) -> None:
    """Reserved hook for version-output validation."""
    versions: dict[str, str] = {}
    for line in content.splitlines():
        if line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        versions[key.strip()] = val.strip()


def extract_tool_versions(
    runtime: str,
    image_tag: str,
    platform: str,
    output_dir: Path,
    config: GuestImageConfig,
    *,
    validate: bool = True,
) -> None:
    """Extract tool versions from rootfs image using config-driven script."""
    version_script = build_version_script(config)
    if not version_script:
        return
    result = run_cmd(
        [runtime, "run", "--rm", "--platform", platform,
         image_tag, "bash", "-c", version_script],
        capture=True,
    )
    versions_path = output_dir / "tool-versions.txt"
    versions_path.write_text(result.stdout)
    if validate:
        _validate_tool_versions(result.stdout, config)


def _cdxgen_command() -> list[str]:
    """Return the configured cdxgen command.

    CI and developer machines can pin this through CAPSEM_CDXGEN_CMD. The
    default uses npm's package runner so the rootfs build does not depend on a
    globally installed binary.
    """
    configured = os.environ.get("CAPSEM_CDXGEN_CMD", "npx --yes @cyclonedx/cdxgen@latest")
    command = shlex.split(configured)
    if not command:
        raise RuntimeError("CAPSEM_CDXGEN_CMD must not be empty")
    return command


def _validate_cyclonedx_obom(path: Path) -> None:
    """Validate the minimal OBOM contract consumed by capsem-admin/service."""
    try:
        document = json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"cdxgen wrote invalid JSON OBOM at {path}: {exc}") from exc
    if document.get("bomFormat") != "CycloneDX":
        raise RuntimeError(f"OBOM {path} must be CycloneDX JSON")
    metadata = document.get("metadata")
    if not isinstance(metadata, dict):
        raise RuntimeError(f"OBOM {path} is missing metadata")
    tools = metadata.get("tools")
    candidates: list[dict[str, Any]] = []
    if isinstance(tools, dict) and isinstance(tools.get("components"), list):
        candidates = [tool for tool in tools["components"] if isinstance(tool, dict)]
    elif isinstance(tools, list):
        candidates = [tool for tool in tools if isinstance(tool, dict)]
    if not any(
        str(tool.get("name", "")).lower() == "cdxgen" and str(tool.get("version", ""))
        for tool in candidates
    ):
        raise RuntimeError(f"OBOM {path} must record cdxgen name and version in metadata.tools")


def generate_cyclonedx_obom(rootfs_tar: Path, output_path: Path, *, repo_root: Path) -> Path:
    """Generate a CycloneDX OS OBOM for the exported rootfs tar.

    The build ledger records declared build inputs. This OBOM is the runtime
    inventory for what actually ended up in the base image.
    """
    import tempfile

    output_path.parent.mkdir(parents=True, exist_ok=True)
    tmp_parent = repo_root / "target" / "tmp"
    tmp_parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="capsem-obom-", dir=tmp_parent) as tmp:
        rootfs_dir = Path(tmp) / "rootfs"
        rootfs_dir.mkdir()
        run_cmd([
            "tar",
            "--exclude=dev/*",
            "--exclude=proc/*",
            "--exclude=sys/*",
            "-xf",
            str(rootfs_tar),
            "-C",
            str(rootfs_dir),
        ])
        run_cmd([
            *_cdxgen_command(),
            "-t",
            "os",
            "-o",
            str(output_path),
            str(rootfs_dir),
        ])
    _validate_cyclonedx_obom(output_path)
    return output_path


def _blake3_hex(path: Path) -> str:
    """Compute BLAKE3 hash of a file, returning the hex digest."""
    import blake3
    hasher = blake3.blake3()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            hasher.update(chunk)
    return hasher.hexdigest()


def _utc_now_iso() -> str:
    return datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z")


def _file_ledger_entry(path: Path, *, base: Path | None = None) -> dict[str, Any]:
    """Return the immutable ledger identity for a file."""
    if not path.is_file():
        raise FileNotFoundError(path)
    display_path = path
    if base is not None:
        try:
            display_path = path.resolve().relative_to(base.resolve())
        except ValueError:
            display_path = path
    return {
        "path": display_path.as_posix(),
        "size": path.stat().st_size,
        "blake3": _blake3_hex(path),
    }


def _directory_file_entries(directory: Path) -> list[dict[str, Any]]:
    """Return sorted per-file ledger entries for a build context."""
    entries: list[dict[str, Any]] = []
    for path in sorted(p for p in directory.rglob("*") if p.is_file()):
        entries.append(_file_ledger_entry(path, base=directory))
    return entries


def _directory_tree_hash(directory: Path) -> str:
    """Hash a directory tree from relative paths and file BLAKE3 hashes."""
    import blake3
    hasher = blake3.blake3()
    for entry in _directory_file_entries(directory):
        hasher.update(entry["path"].encode())
        hasher.update(b"\0")
        hasher.update(str(entry["size"]).encode())
        hasher.update(b"\0")
        hasher.update(entry["blake3"].encode())
        hasher.update(b"\n")
    return hasher.hexdigest()


def _git_revision(repo_root: Path) -> str | None:
    try:
        result = run_cmd(
            ["git", "rev-parse", "HEAD"],
            cwd=repo_root,
            capture=True,
            echo=False,
        )
        return result.stdout.strip() or None
    except Exception:
        return None


def _project_version_or_unknown(repo_root: Path) -> str:
    try:
        return get_project_version(repo_root)
    except Exception:
        return "unknown"


def _append_build_ledger(arch_output: Path, record: dict[str, Any]) -> Path:
    """Append one JSON record to the per-arch build ledger."""
    arch_output.mkdir(parents=True, exist_ok=True)
    ledger_path = arch_output / BUILD_LEDGER_NAME
    full_record = {
        "schema": "capsem.build_ledger.v1",
        "timestamp": _utc_now_iso(),
        **record,
    }
    with ledger_path.open("a") as f:
        f.write(json.dumps(full_record, sort_keys=True) + "\n")
    return ledger_path


def _build_input_record(
    *,
    repo_root: Path,
    arch_name: str,
    template: str,
    template_name: str,
    context_dir: Path,
    dockerfile_path: Path,
    docker_tag: str,
    docker_platform: str,
    runtime: str,
) -> dict[str, Any]:
    return {
        "arch": arch_name,
        "template": template,
        "template_name": template_name,
        "runtime": runtime,
        "docker_tag": docker_tag,
        "docker_platform": docker_platform,
        "project_version": _project_version_or_unknown(repo_root),
        "git_revision": _git_revision(repo_root),
        "dockerfile": _file_ledger_entry(dockerfile_path, base=context_dir),
        "build_context": {
            "hash": _directory_tree_hash(context_dir),
            "files": _directory_file_entries(context_dir),
        },
    }


def _path_input_record(path_value: str | None) -> dict[str, Any] | None:
    """Return debug identity for a profile-provided path when it exists."""
    if not path_value:
        return None
    path = Path(path_value)
    record: dict[str, Any] = {"path": path.as_posix()}
    if path.is_file():
        record["file"] = _file_ledger_entry(path)
    elif path.is_dir():
        record["directory"] = {
            "hash": _directory_tree_hash(path),
            "files": _directory_file_entries(path),
        }
    else:
        record["exists"] = False
    return record


def _package_config_record(config: GuestImageConfig) -> dict[str, Any]:
    """Record declared package config inputs, not installed package state."""
    package_inputs: dict[str, Any] = {}
    for key, package_set in sorted(config.package_sets.items()):
        package_inputs[key] = {
            "manager": package_set.manager.value,
            "install_cmd": package_set.install_cmd,
            "packages": list(package_set.packages),
            "version_commands": dict(sorted(package_set.version_commands.items())),
        }
    return package_inputs


def _rootfs_config_input_record(
    config: GuestImageConfig,
    arch_name: str,
) -> dict[str, Any]:
    """Build the rootfs debug ledger record for declared config inputs.

    This record is intentionally not an installed-package ledger. Installed
    package/component truth belongs to the CycloneDX OBOM generated from the
    produced rootfs. The build ledger records the config and profile inputs we
    fed into the build so failures can be retraced.
    """
    ctx = _rootfs_context(config, arch_name)
    erofs = config.build.erofs
    return {
        "stage": "rootfs.config_inputs",
        "arch": arch_name,
        "package_inputs": _package_config_record(config),
        "rendered_rootfs_inputs": {
            "apt_packages": list(ctx["apt_packages"]),
            "python_packages": list(ctx["python_packages"]),
            "python_install_cmd": ctx["python_install_cmd"],
            "npm_packages": list(ctx["npm_packages"]),
            "npm_prefix": ctx["npm_prefix"],
            "curl_installs": list(ctx["curl_installs"]),
        },
        "profile_inputs": {
            "root_seed": {
                "enabled": config.profile_root_seed,
                "source": _path_input_record(config.profile_root_seed_path),
            },
            "build_script": {
                "enabled": config.profile_build_script,
                "source": _path_input_record(config.profile_build_script_path),
            },
        },
        "erofs": {
            "enabled": erofs.enabled,
            "compression": erofs.compression.value,
            "compression_level": erofs.compression_level,
            "cluster_size": erofs.cluster_size,
        },
    }


def _select_rootfs_asset(asset_dir: Path) -> str | None:
    """Return the canonical rootfs asset name for a directory."""
    for filename in ROOTFS_ASSET_PREFERENCE:
        if (asset_dir / filename).is_file():
            return filename
    return None


def _next_or_existing_asset_version(
    output_dir: Path,
    date_prefix: str,
    arch_assets: dict[str, dict[str, dict]],
) -> str:
    manifest_path = output_dir / "manifest.json"
    patch = 1
    if not manifest_path.is_file():
        return f"{date_prefix}.{patch}"
    try:
        existing = json.loads(manifest_path.read_text())
    except json.JSONDecodeError:
        return f"{date_prefix}.{patch}"
    assets = existing.get("assets", {})
    releases = assets.get("releases", {})
    current = assets.get("current")
    if current in releases and releases[current].get("arches", {}) == arch_assets:
        return current
    for version in releases:
        if not version.startswith(f"{date_prefix}."):
            continue
        try:
            patch = max(patch, int(version.rsplit(".", 1)[1]) + 1)
        except ValueError:
            continue
    return f"{date_prefix}.{patch}"


def generate_checksums(output_dir: Path, version: str) -> Path:
    """Generate BLAKE3 checksums and manifest.json for all assets."""
    # Collect all asset files across arch subdirs
    arch_dirs = [d for d in output_dir.iterdir() if d.is_dir() and d.name != "current"]
    all_files: list[str] = []
    for arch_dir in sorted(arch_dirs):
        arch_name = arch_dir.name
        for filename in BOOT_ASSETS:
            if (arch_dir / filename).is_file():
                all_files.append(f"{arch_name}/{filename}")
        if rootfs_name := _select_rootfs_asset(arch_dir):
            all_files.append(f"{arch_name}/{rootfs_name}")
        elif any((arch_dir / filename).is_file() for filename in BOOT_ASSETS):
            raise FileNotFoundError(f"{arch_dir / 'rootfs.erofs'}")
        if (arch_dir / OBOM_ASSET).is_file():
            all_files.append(f"{arch_name}/{OBOM_ASSET}")

    if not all_files:
        # Flat layout fallback
        for f in BOOT_ASSETS:
            if (output_dir / f).is_file():
                all_files.append(f)
        if rootfs_name := _select_rootfs_asset(output_dir):
            all_files.append(rootfs_name)
        elif all_files:
            raise FileNotFoundError(f"{output_dir / 'rootfs.erofs'}")
        if (output_dir / OBOM_ASSET).is_file():
            all_files.append(OBOM_ASSET)

    # Compute BLAKE3 hashes using Python blake3 library.
    b3sums_lines = []
    hashes: dict[str, str] = {}
    for filepath in all_files:
        full_path = output_dir / filepath
        b3hash = _blake3_hex(full_path)
        hashes[filepath] = b3hash
        b3sums_lines.append(f"{b3hash}  {filepath}")
    (output_dir / "B3SUMS").write_text("\n".join(b3sums_lines) + "\n")

    arch_assets: dict[str, dict[str, dict]] = {}
    for filepath in all_files:
        full_path = output_dir / filepath
        b3hash = hashes[filepath]
        size = full_path.stat().st_size

        if "/" in filepath:
            arch_name, filename = filepath.split("/", 1)
        else:
            arch_name = "unknown"
            filename = filepath

        arch_assets.setdefault(arch_name, {})[filename] = {
            "hash": b3hash, "size": size,
        }

    # Build v2 manifest with separate assets/binaries sections. Reuse the
    # current release for identical assets so dev initrd repacks do not mint
    # endless no-op asset versions.
    import datetime
    today = datetime.date.today()
    date_prefix = today.strftime("%Y.%m%d")
    asset_version = _next_or_existing_asset_version(
        output_dir,
        date_prefix,
        arch_assets,
    )

    manifest = {
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": asset_version,
            "releases": {
                asset_version: {
                    "date": today.isoformat(),
                    "deprecated": False,
                    "min_binary": "1.0.0",
                    "arches": arch_assets,
                },
            },
        },
        "binaries": {
            "current": version,
            "releases": {
                version: {
                    "date": today.isoformat(),
                    "deprecated": False,
                    "min_assets": asset_version,
                },
            },
        },
    }
    manifest_path = output_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")

    # Create assets/current symlink pointing to the most recently built arch.
    # Tauri bundle resources reference assets/current/ so they resolve on any platform.
    current_link = output_dir / "current"
    if arch_dirs:
        target = sorted(arch_dirs)[-1].name
        if current_link.is_symlink() or current_link.is_file():
            current_link.unlink()
        elif current_link.is_dir():
            shutil.rmtree(current_link)
        current_link.symlink_to(target)

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
    guest_dir = Path(config.guest_dir_path) if config.guest_dir_path else repo_root / "guest"
    # Render Dockerfile
    dockerfile_content = render_dockerfile(template_name, config, arch_name, **kwargs)
    dockerfile_path = context_dir / "Dockerfile"
    dockerfile_path.write_text(dockerfile_content)

    if "rootfs" in template_name:
        # CA cert
        shutil.copy2(
            str(repo_root / "security" / "keys" / "capsem-ca.crt"),
            str(context_dir / "capsem-ca.crt"),
        )
        artifacts = guest_dir / "artifacts"
        for name in ("capsem-bashrc", "banner.txt", "tips.txt"):
            shutil.copy2(
                str(artifacts / name),
                str(context_dir / name),
            )
        # Diagnostics
        diag_src = artifacts / "diagnostics"
        diag_dst = context_dir / "diagnostics"
        if diag_src.is_dir():
            shutil.copytree(str(diag_src), str(diag_dst), dirs_exist_ok=True)
        # Rootfs artifact scripts (doctor, bench, snapshots, etc.)
        for name in ROOTFS_SCRIPTS:
            src = artifacts / name
            if src.is_file():
                shutil.copy2(str(src), str(context_dir / name))
        # Script directories
        for name in ROOTFS_SCRIPT_DIRS:
            src = artifacts / name
            if src.is_dir():
                shutil.copytree(str(src), str(context_dir / name), dirs_exist_ok=True)
        if config.profile_root_seed:
            if not config.profile_root_seed_path:
                raise FileNotFoundError("profile_root_seed_path")
            profile_root = Path(config.profile_root_seed_path)
            if not profile_root.is_dir():
                raise FileNotFoundError(profile_root)
            shutil.copytree(
                str(profile_root),
                str(context_dir / "profile-root"),
                dirs_exist_ok=True,
            )
        if config.profile_build_script:
            if not config.profile_build_script_path:
                raise FileNotFoundError("profile_build_script_path")
            profile_build = Path(config.profile_build_script_path)
            if not profile_build.is_file():
                raise FileNotFoundError(profile_build)
            shutil.copy2(str(profile_build), str(context_dir / "profile-build.sh"))
        # Agent binaries (if they exist in context already from cross_compile_agent)
        # They may have been copied to context_dir by the pipeline before this call

    elif "kernel" in template_name:
        # Defconfig -- preserve directory structure for COPY {{ arch.defconfig }}
        arch = config.build.architectures[arch_name]
        defconfig_src = guest_dir / "config" / arch.defconfig
        defconfig_dst = context_dir / arch.defconfig
        defconfig_dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(str(defconfig_src), str(defconfig_dst))
        # capsem-init
        shutil.copy2(
            str(guest_dir / "artifacts" / "capsem-init"),
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

    # Sync container VM clock with host to prevent apt date errors
    sync_container_clock()

    # Doctor check: cross-compilation target (skip on macOS -- container handles it)
    if template == "rootfs" and sys.platform != "darwin":
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

    # Use a temporary directory inside the project root's target/ folder.
    # On macOS, system temp dirs (/var/folders) are often not mountable by Docker/Colima.
    build_tmp = repo_root / "target" / "tmp"
    build_tmp.mkdir(parents=True, exist_ok=True)
    
    with tempfile.TemporaryDirectory(prefix=f"capsem-build-{template}-", dir=build_tmp) as tmpdir:
        context_dir = Path(tmpdir)

        if template == "kernel":
            # Resolve kernel version
            if kernel_version is None:
                kernel_version = resolve_kernel_version(arch.kernel_branch)
            print(f"Kernel: {kernel_version}")

            dockerfile_path = prepare_build_context(
                config, arch_name, template_name, context_dir, repo_root,
                kernel_version=kernel_version,
            )
            build_inputs = _build_input_record(
                repo_root=repo_root,
                arch_name=arch_name,
                template=template,
                template_name=template_name,
                context_dir=context_dir,
                dockerfile_path=dockerfile_path,
                docker_tag=tag,
                docker_platform=arch.docker_platform,
                runtime=runtime,
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
            remove_image(runtime, tag)
            _append_build_ledger(arch_output, {
                "stage": "kernel.assets",
                "inputs": build_inputs,
                "kernel_version": kernel_version,
                "outputs": [
                    _file_ledger_entry(vmlinuz, base=arch_output),
                    _file_ledger_entry(initrd, base=arch_output),
                ],
            })
            print(f"  vmlinuz:    {vmlinuz}")
            print(f"  initrd.img: {initrd}")

        elif template == "rootfs":
            # Cross-compile agent binaries
            print(f"Cross-compiling guest binaries for {arch.rust_target}...")
            binaries = cross_compile_agent(arch.rust_target, repo_root, context_dir)
            for b in binaries:
                print(f"  {b.name}: {b.stat().st_size} bytes")

            dockerfile_path = prepare_build_context(
                config, arch_name, template_name, context_dir, repo_root,
            )
            build_inputs = _build_input_record(
                repo_root=repo_root,
                arch_name=arch_name,
                template=template,
                template_name=template_name,
                context_dir=context_dir,
                dockerfile_path=dockerfile_path,
                docker_tag=tag,
                docker_platform=arch.docker_platform,
                runtime=runtime,
            )
            _append_build_ledger(
                arch_output,
                _rootfs_config_input_record(config, arch_name),
            )
            docker_build(
                runtime, tag, context_dir / "Dockerfile", context_dir,
                arch.docker_platform, ci_cache=ci,
            )

            # Export and compress
            tar_path = arch_output / "rootfs.tar"
            print("Exporting rootfs filesystem...")
            export_container_fs(runtime, tag, arch.docker_platform, tar_path)
            tar_entry = _file_ledger_entry(tar_path, base=arch_output)
            _append_build_ledger(arch_output, {
                "stage": "rootfs.export",
                "inputs": build_inputs,
                "intermediates": [tar_entry],
            })

            erofs_enabled, erofs_compression, erofs_cluster_size, erofs_level = (
                experimental_erofs_build_config(defaults=config.build.erofs)
            )
            if not erofs_enabled:
                raise ValueError("EROFS build cannot be disabled for the 1.3 asset contract")
            erofs_path = arch_output / "rootfs.erofs"
            print(
                f"Creating EROFS ({erofs_compression} compression"
                f"{', level ' + erofs_level if erofs_level else ''}"
                f"{', cluster ' + erofs_cluster_size if erofs_cluster_size else ''})..."
            )
            create_erofs(
                runtime, tar_path, erofs_path,
                erofs_compression,
                erofs_cluster_size,
                erofs_level,
            )
            erofs_entry = _file_ledger_entry(erofs_path, base=arch_output)
            _append_build_ledger(arch_output, {
                "stage": "rootfs.erofs",
                "inputs": build_inputs,
                "intermediates": [tar_entry],
                "erofs": {
                    "compression": erofs_compression,
                    "compression_level": erofs_level,
                    "cluster_size": erofs_cluster_size,
                    "utils_image": erofs_utils_image_for(erofs_compression),
                },
                "outputs": [erofs_entry],
            })
            print("Generating CycloneDX OBOM...")
            obom_path = arch_output / OBOM_ASSET
            generate_cyclonedx_obom(tar_path, obom_path, repo_root=repo_root)
            obom_entry = _file_ledger_entry(obom_path, base=arch_output)
            _append_build_ledger(arch_output, {
                "stage": "rootfs.obom",
                "inputs": build_inputs,
                "intermediates": [tar_entry],
                "generator": "cdxgen",
                "outputs": [obom_entry],
            })
            tar_path.unlink(missing_ok=True)

            print("Extracting tool versions...")
            extract_tool_versions(runtime, tag, arch.docker_platform, arch_output, config)
            versions_path = arch_output / "tool-versions.txt"
            if versions_path.is_file():
                _append_build_ledger(arch_output, {
                    "stage": "rootfs.tool_versions",
                    "inputs": build_inputs,
                    "outputs": [_file_ledger_entry(versions_path, base=arch_output)],
                })
            remove_image(runtime, tag)

            print(f"  rootfs.erofs:    {erofs_path}")


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

    # Prune dangling images left by multi-stage builds
    runtime = detect_runtime()
    try:
        run_cmd([runtime, "image", "prune", "-f"], capture=True)
        print("Pruned dangling images.")
    except RuntimeError:
        pass

    version = get_project_version(repo_root)
    print(f"\nGenerating checksums (version {version})...")
    generate_checksums(output_dir, version)

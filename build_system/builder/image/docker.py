"""Dockerfile generation and build execution from GuestImageConfig.

Renders Dockerfiles via Jinja2 templates and executes Docker/Podman builds
to produce VM boot assets. Supports multi-architecture output (arm64, x86_64).
"""

from __future__ import annotations

import contextlib
import datetime
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import uuid
from pathlib import Path, PurePosixPath
from typing import Any

from jinja2 import Environment, FileSystemLoader

from ..gate import auditfs
from ..policy.dockerpolicy import (
    BuildNetwork,
    ContainerNetwork,
    require_build_network,
    require_container_network,
)
from ..release.obom import validate_exported_rootfs_obom
from . import assetdependencies, guestbuilder
from .assettools import image_tag as asset_tools_image_tag
from .doctor import check_container_runtime
from .guestbuilder import image_tag
from .models import BuildConfig, ErofsConfig, GuestImageConfig, RootfsConfig

TEMPLATES_DIR = Path(__file__).resolve().parents[3] / "config" / "docker"
CLOCK_SYNC_SCRIPT = Path(__file__).resolve().parents[3] / "build_system" / "scripts" / "build" / "sync-container-clock.py"
BOOT_ASSETS = ("vmlinuz", "initrd.img")
ROOTFS_ASSET_PREFERENCE = ("rootfs.erofs",)
OBOM_ASSET = "obom.cdx.json"
SOFTWARE_INVENTORY_ASSET = "software-inventory.json"
BUILD_LEDGER_NAME = "build-ledger.log"
CONTAINER_PROBE_TIMEOUT_SECONDS = 60
CONTAINER_PROBE_CLEANUP_TIMEOUT_SECONDS = 15
OBOM_COMMAND_TIMEOUT_SECONDS = 600

# Guest binaries COPY'd into the rootfs (cross-compiled Rust binaries).
GUEST_BINARIES = [
    "capsem-pty-agent",
    "capsem-net-proxy",
    "capsem-dns-proxy",
    "capsem-mcp-server",
    "capsem-sysutil",
    "capsem-bench-rs",
]

GUEST_BINARY_SOURCES = {}

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
    """Finalize guest binaries as host-owned atomic 0555 files.

    Docker-for-Mac bind-mount metadata can arrive after the container exits and
    overwrite a host chmod on the same inode. Copying into a host-created
    temporary inode and replacing the bind-mounted output makes that delayed
    metadata update harmless.
    """
    for p in paths:
        if not p.exists():
            raise FileNotFoundError(p)
        staged = p.with_name(f".{p.name}.readonly-{os.getpid()}")
        try:
            shutil.copyfile(p, staged)
            os.chmod(staged, 0o555)
            os.replace(staged, p)
        finally:
            staged.unlink(missing_ok=True)
        mode = p.stat().st_mode & 0o777
        if mode != 0o555:
            raise RuntimeError(f"guest binary {p} expected mode 0555, got {oct(mode)}")


def _guest_binary_source(binary: str) -> str:
    return GUEST_BINARY_SOURCES.get(binary, binary)


def _debian_snapshot_context(config: GuestImageConfig) -> dict[str, str]:
    """One checked-in Debian package authority for every asset helper."""
    settings = config.build.asset_tools
    return {
        "debian_snapshot_base": settings.debian_snapshot_base,
        "debian_security_snapshot_base": settings.debian_security_snapshot_base,
        "debian_snapshot_id": settings.debian_snapshot_id,
    }


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
    return {
        **_debian_snapshot_context(config),
        "arch": arch,
        "arch_name": arch_name,
        "apt_packages": apt_packages,
        "python_packages": python_packages,
        "python_install_cmd": python_install_cmd,
        "npm_packages": npm_packages,
        "npm_prefix": npm_prefix,
        "dependency_artifacts": config.build.asset_dependencies.architectures[arch_name],
        "guest_binaries": GUEST_BINARIES,
        "profile_root_seed": config.profile_root_seed,
        "profile_build_script": config.profile_build_script,
    }


def _kernel_context(config: GuestImageConfig, arch_name: str) -> dict[str, Any]:
    """Build Jinja context for Dockerfile.kernel.j2."""
    arch = config.build.architectures[arch_name]
    return {
        **_debian_snapshot_context(config),
        "arch": arch,
        "arch_name": arch_name,
        "kernel_version": config.build.kernel.version,
        "kernel_sha256": config.build.kernel.sha256,
    }


def generate_build_context(
    template_name: str,
    config: GuestImageConfig,
    arch_name: str,
) -> dict[str, Any]:
    """Generate the Jinja template context dict for a given template.

    Args:
        template_name: Template filename (e.g., "Dockerfile.rootfs.j2").
        config: Guest image configuration.
        arch_name: Architecture name (e.g., "arm64", "x86_64").
    Returns:
        Context dict ready for Jinja rendering.

    Raises:
        ValueError: If template_name is not recognized.
        KeyError: If arch_name is not in config.build.architectures.
    """
    if template_name in {
        "Dockerfile.rootfs.j2",
        config.build.asset_dependencies.rootfs_template,
    }:
        ctx = _rootfs_context(config, arch_name)
    elif template_name in {
        "Dockerfile.kernel.j2",
        config.build.asset_dependencies.kernel_template,
    }:
        ctx = _kernel_context(config, arch_name)
    else:
        raise ValueError(f"Unknown template: {template_name}")

    return ctx


def render_dockerfile(
    template_name: str,
    config: GuestImageConfig,
    arch_name: str,
) -> str:
    """Render a Dockerfile from a Jinja2 template with config context.

    Args:
        template_name: Template filename (e.g., "Dockerfile.rootfs.j2").
        config: Guest image configuration.
        arch_name: Architecture name (e.g., "arm64", "x86_64").
    Returns:
        Rendered Dockerfile as a string.

    Raises:
        ValueError: If template_name is not recognized.
        KeyError: If arch_name is not in config.build.architectures.
    """
    context = generate_build_context(template_name, config, arch_name)
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
    timeout: float | None = None,
) -> subprocess.CompletedProcess[str]:
    """Run a subprocess command. Single mock seam for tests."""
    if echo:
        print(f"  -> {' '.join(str(c) for c in cmd)}")
    kwargs: dict[str, Any] = {"check": True, "text": True}
    if cwd:
        kwargs["cwd"] = str(cwd)
    if capture:
        kwargs["capture_output"] = True
    if timeout is not None:
        kwargs["timeout"] = timeout
    try:
        return subprocess.run(cmd, **kwargs)
    except subprocess.CalledProcessError as error:
        if capture:
            if error.stdout:
                print(
                    error.stdout, file=sys.stderr, end="" if error.stdout.endswith("\n") else "\n"
                )
            if error.stderr:
                print(
                    error.stderr, file=sys.stderr, end="" if error.stderr.endswith("\n") else "\n"
                )
        raise


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

    This sets the VM clock to the current host UTC time before builds. The
    shared primitive owns a hard timeout; failures abort before an expensive
    build instead of leaving Docker clients blocked indefinitely.
    """
    if sys.platform != "darwin":
        return

    run_cmd(
        [sys.executable, str(CLOCK_SYNC_SCRIPT)],
        capture=True,
        echo=False,
        timeout=15,
    )


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
    with contextlib.suppress(RuntimeError):
        run_cmd([runtime, "rmi", "-f", tag], capture=True)


def docker_build(
    runtime: str,
    tag: str,
    dockerfile_path: str | Path,
    context_dir: str | Path,
    platform: str,
    *,
    network: BuildNetwork,
    build_args: dict[str, str] | None = None,
    ci_cache: bool = False,
) -> None:
    """Build a container image."""
    network_value = require_build_network(network)
    args_flags: list[str] = []
    for k, v in (build_args or {}).items():
        args_flags.extend(["--build-arg", f"{k}={v}"])

    if ci_cache:
        run_cmd(
            [
                "docker",
                "buildx",
                "build",
                "--platform",
                platform,
                "--network",
                network_value,
                "--cache-from",
                f"type=gha,scope={tag}",
                "--cache-to",
                f"type=gha,mode=max,scope={tag}",
                "--load",
                *args_flags,
                "-t",
                tag,
                "-f",
                str(dockerfile_path),
                str(context_dir),
            ]
        )
    else:
        run_cmd(
            [
                runtime,
                "build",
                "--platform",
                platform,
                "--network",
                network_value,
                *args_flags,
                "-t",
                tag,
                "-f",
                str(dockerfile_path),
                str(context_dir),
            ]
        )


def _asset_dependency_template(config: GuestImageConfig, template: str) -> str:
    settings = config.build.asset_dependencies
    if template == "rootfs":
        return settings.rootfs_template
    if template == "kernel":
        return settings.kernel_template
    raise ValueError(f"unsupported asset dependency template: {template}")


def _asset_dependency_tag(
    config: GuestImageConfig,
    arch_name: str,
    template: str,
) -> str:
    rendered = render_dockerfile(
        _asset_dependency_template(config, template),
        config,
        arch_name,
    )
    return assetdependencies.image_tag(
        config,
        arch_name,
        template,
        rendered.encode(),
    )


def require_asset_dependencies(
    runtime: str,
    config: GuestImageConfig,
    arch_name: str,
    template: str,
) -> assetdependencies.AssetDependencyImage:
    """Bind one input-keyed runnable reference to its exact local image ID."""
    tag = _asset_dependency_tag(config, arch_name, template)
    arch = config.build.architectures[arch_name]
    result = run_cmd(
        [
            runtime,
            "image",
            "inspect",
            "--format",
            (
                "{{.Os}}/{{.Architecture}}\t{{.Id}}\t"
                '{{ index .Config.Labels "org.capsem.asset-dependencies.input-key" }}'
            ),
            tag,
        ],
        capture=True,
    )
    try:
        platform, image_id, label = result.stdout.strip().split("\t")
    except ValueError as error:
        raise RuntimeError(f"asset dependency image inspection was malformed: {tag}") from error
    if platform != arch.docker_platform:
        raise RuntimeError(
            f"asset dependency image platform mismatch: {tag} is {platform}, "
            f"expected {arch.docker_platform}"
        )
    if label != tag:
        raise RuntimeError(f"asset dependency image identity mismatch: {tag}")
    if not image_id.startswith("sha256:"):
        raise RuntimeError(f"asset dependency image has no exact ID: {tag}")
    return assetdependencies.AssetDependencyImage(reference=tag, image_id=image_id)


def materialize_asset_dependencies(
    config: GuestImageConfig,
    arch_name: str,
    *,
    template: str,
    repo_root: Path | None = None,
) -> assetdependencies.AssetDependencyImage:
    """Build one input-keyed network-open helper before the sealed source lane."""
    import tempfile

    if repo_root is None:
        repo_root = Path.cwd()
    runtime = detect_runtime()
    tag = _asset_dependency_tag(config, arch_name, template)
    try:
        run_cmd([runtime, "image", "inspect", tag], capture=True)
    except subprocess.CalledProcessError:
        pass
    else:
        return require_asset_dependencies(runtime, config, arch_name, template)

    arch = config.build.architectures[arch_name]
    build_tmp = repo_root / "cache" / "target" / "tmp"
    build_tmp.mkdir(parents=True, exist_ok=True)
    dependency_template = _asset_dependency_template(config, template)
    with tempfile.TemporaryDirectory(
        prefix=f"capsem-{template}-dependencies-",
        dir=build_tmp,
    ) as tmpdir:
        context_dir = Path(tmpdir)
        dockerfile = prepare_build_context(
            config,
            arch_name,
            dependency_template,
            context_dir,
            repo_root,
        )
        docker_build(
            runtime,
            tag,
            dockerfile,
            context_dir,
            arch.docker_platform,
            network=config.build.materialize_network,
            build_args={
                "BASE": arch.base_image,
                "TRUSTSTORE_IMAGE": arch.rust_builder_base_image,
                "INPUT_IDENTITY": tag,
            },
            ci_cache=is_ci(),
        )
    return require_asset_dependencies(runtime, config, arch_name, template)


def extract_kernel_assets(
    runtime: str,
    image_tag: str,
    platform: str,
    output_dir: Path,
) -> tuple[Path, Path]:
    """Extract vmlinuz and initrd.img from a kernel builder image."""
    output_dir.mkdir(parents=True, exist_ok=True)
    result = run_cmd(
        [
            runtime,
            "create",
            "--pull",
            "never",
            "--network",
            "none",
            "--platform",
            platform,
            image_tag,
            "/bin/true",
        ],
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
        [
            runtime,
            "create",
            "--pull",
            "never",
            "--network",
            "none",
            "--platform",
            platform,
            image_tag,
            "/bin/true",
        ],
        capture=True,
    )
    cid = result.stdout.strip()
    try:
        run_cmd([runtime, "export", cid, "-o", str(output_tar)])
    finally:
        run_cmd([runtime, "rm", cid])


def _native_linux_platform() -> str:
    """Return the Linux container platform matching the Docker host CPU."""
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return "linux/amd64"
    if machine in {"arm64", "aarch64"}:
        return "linux/arm64"
    raise RuntimeError(f"unsupported Docker host architecture: {machine}")


def create_erofs(
    runtime: str,
    tar_path: Path,
    output_path: Path,
    compression: str,
    cluster_size: str | None = None,
    compression_level: str | None = None,
    *,
    tool_image: str,
    runtime_network: ContainerNetwork,
) -> None:
    """Create an EROFS image from a tar archive using a container."""
    network_value = require_container_network(runtime_network)
    if compression not in {"lz4", "lz4hc"}:
        raise ValueError(f"unsupported EROFS compression: {compression}")

    if compression_level is not None:
        level = int(compression_level)
        if compression == "lz4":
            raise ValueError("lz4 EROFS compression does not accept a level")
        if compression == "lz4hc" and not 0 <= level <= 12:
            raise ValueError("lz4hc EROFS compression level must be between 0 and 12")

    tar_abs = tar_path.resolve()
    output_abs = output_path.resolve()
    common_dir = Path(os.path.commonpath([tar_abs.parent, output_abs.parent]))
    tar_rel = tar_abs.relative_to(common_dir).as_posix()
    out_rel = output_abs.relative_to(common_dir).as_posix()
    out_dir = Path(out_rel).parent.as_posix()
    cluster_flag = f" -C{cluster_size}" if cluster_size else ""
    level_flag = f",level={compression_level}" if compression_level else ""
    mkdir_output = "" if out_dir == "." else f"mkdir -p /assets/{out_dir} && "
    host_uid = os.getuid()
    host_gid = os.getgid()
    host_platform = _native_linux_platform()

    run_cmd(
        [
            runtime,
            "run",
            "--rm",
            "--pull",
            "never",
            "--network",
            network_value,
            "--platform",
            host_platform,
            "-v",
            f"{common_dir}:/assets",
            tool_image,
            "bash",
            "-c",
            f"mkdir /rootfs && {mkdir_output}tar xf /assets/{tar_rel} -C /rootfs && "
            f"mkfs.erofs -Enosbcrc -z{compression}{level_flag}{cluster_flag} "
            f"/assets/{out_rel} /rootfs && "
            f"chown {host_uid}:{host_gid} /assets/{out_rel}",
        ],
        capture=True,
    )


def validate_rootfs_export(tar_path: Path, limits: RootfsConfig) -> None:
    """Reject an oversized or compositionally forbidden exported rootfs."""
    size = tar_path.stat().st_size
    if size > limits.max_uncompressed_bytes:
        raise ValueError(
            "uncompressed rootfs is "
            f"{size} bytes, above configured maximum {limits.max_uncompressed_bytes}"
        )
    with tarfile.open(tar_path, mode="r") as archive:
        for member in archive:
            name = member.name
            while name.startswith("./"):
                name = name[2:]
            path = PurePosixPath(name)
            if path.is_absolute() or any(part == ".." for part in path.parts):
                raise ValueError(f"unsafe rootfs archive member: {member.name}")
            for prefix in limits.forbidden_path_prefixes:
                if name.startswith(prefix):
                    raise ValueError(f"forbidden rootfs payload survived cleanup: {name}")


def validate_erofs_size(image_path: Path, limits: RootfsConfig) -> None:
    """Reject a publishable EROFS whose packed bytes exceed its growth budget."""
    size = image_path.stat().st_size
    if size > limits.max_erofs_bytes:
        raise ValueError(
            f"EROFS rootfs is {size} bytes, above configured maximum {limits.max_erofs_bytes}"
        )


def _native_build_arch(config: GuestImageConfig) -> str:
    """Return the configured name matching the Docker host architecture."""
    platform_name = _native_linux_platform()
    matches = [
        name
        for name, arch in config.build.architectures.items()
        if arch.docker_platform == platform_name
    ]
    if len(matches) != 1:
        raise RuntimeError(
            f"expected one guest architecture for Docker host {platform_name}, got {matches}"
        )
    return matches[0]


def _asset_tools_image(config: GuestImageConfig, repo_root: Path) -> str:
    """Return the input-keyed helper the gate must materialize first."""
    return asset_tools_image_tag(config.build, _native_build_arch(config), repo_root)


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
    compression = source.get("CAPSEM_BUILD_EROFS_COMPRESSION") or (
        defaults.compression.value if defaults is not None else "lz4hc"
    )
    if compression not in {"lz4", "lz4hc"}:
        raise ValueError("CAPSEM_BUILD_EROFS_COMPRESSION must be one of: lz4, lz4hc")
    cluster_size = source.get("CAPSEM_BUILD_EROFS_CLUSTER_SIZE") or (
        str(defaults.cluster_size) if defaults is not None and defaults.cluster_size else None
    )
    if cluster_size is not None:
        try:
            normalized_cluster_size = int(cluster_size)
            ErofsConfig(cluster_size=normalized_cluster_size)
        except (TypeError, ValueError) as exc:
            raise ValueError(
                "CAPSEM_BUILD_EROFS_CLUSTER_SIZE must be a power of two between 4096 and 1048576"
            ) from exc
        cluster_size = str(normalized_cluster_size)
    compression_level = source.get("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL") or (
        str(defaults.compression_level)
        if defaults is not None and defaults.compression_level is not None
        else None
    )
    if compression_level is not None:
        level = int(compression_level)
        if compression == "lz4":
            raise ValueError("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL is not valid for lz4")
        if compression == "lz4hc" and not 0 <= level <= 12:
            raise ValueError("CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL must be 0..12 for lz4hc")
    return enabled, compression, cluster_size, compression_level


def container_compile_agent(
    build: BuildConfig,
    arch_name: str,
    repo_root: Path,
    output_dir: Path,
) -> list[Path]:
    """Compile guest agent binaries inside a Linux container.

    Used on every host and target. The dependency/toolchain image is
    materialized before this call; the build itself has no network or pull.
    """
    resolved = guestbuilder.environment(build, arch_name)
    rust_target = resolved.rust_target
    runtime = detect_runtime()
    # The host's platform, even for a foreign target: that target is
    # cross-compiled in a host-platform image rather than emulated in its own.
    docker_platform = resolved.docker_platform
    output_dir.mkdir(parents=True, exist_ok=True)

    # Build all shell commands from GUEST_BINARIES constant
    cp_cmds = " && ".join(
        f"cp cache/target/{rust_target}/release/{_guest_binary_source(b)} /output/{b}"
        for b in GUEST_BINARIES
    )
    rm_cmds = " ".join(f"/output/{b}" for b in GUEST_BINARIES)
    chmod_cmds = " ".join(f"/output/{b}" for b in GUEST_BINARIES)
    file_cmds = " && ".join(f"ls -l /output/{b}" for b in GUEST_BINARIES)

    image = image_tag(build, arch_name, repo_root)
    try:
        inspected = run_cmd(
            [runtime, "image", "inspect", "--format", "{{.Os}}/{{.Architecture}}", image],
            capture=True,
            echo=False,
        )
    except subprocess.CalledProcessError as error:
        raise RuntimeError(
            f"locked guest Rust builder is missing: {image}; "
            "materialize the asset build inputs before cross-compiling"
        ) from error
    found_platform = inspected.stdout.strip()
    if found_platform != docker_platform:
        raise RuntimeError(
            f"locked guest Rust builder {image} resolves to {found_platform or '<empty>'}, "
            f"expected {docker_platform}"
        )

    # The container owns its toolchain settings, passed in rather than read out
    # of the tree -- see the workspace comment below for why `.cargo/config.toml`
    # must not reach `/build`. `ring` is the only crate in the guest graph that
    # compiles C, and the image materialized clang for it.
    cross_env: list[str] = []
    if resolved.cross:
        variable = rust_target.replace("-", "_")
        cross_env = [
            "-e",
            f"CC_{variable}=clang",
            "-e",
            f"CFLAGS_{variable}=--target={rust_target}",
            "-e",
            f"CARGO_TARGET_{variable.upper()}_LINKER=rust-lld",
        ]

    shape = "cross" if resolved.cross else "native"
    print(f"  Container build ({docker_platform}, {shape} -> {rust_target}) ...")
    # Source is mounted :ro to protect the host. We symlink everything into
    # a writable /build dir, while Cargo.lock stays the checked-in read-only
    # input. The image owns the registry and rustup trees; masking either with
    # an anonymous volume would turn a cold run back into a network build.
    run_cmd(
        [
            runtime,
            "run",
            "--rm",
            "--pull",
            "never",
            "--network",
            build.guest_rust_builder.runtime_network,
            "--platform",
            docker_platform,
            "-v",
            f"{repo_root.resolve()}:/src:ro",
            "-v",
            f"{output_dir.resolve()}:/output",
            "-v",
            "/build/target",
            "-w",
            "/build",
            *cross_env,
            image,
            "sh",
            "-c",
            # `/src/*` does not match dotfiles, and that exclusion is
            # load-bearing rather than incidental -- it was relied on for a
            # long time without being stated, so it is stated here.
            #
            # `.cargo/config.toml` declares `linker = "rust-lld"` for
            # `x86_64-unknown-linux-musl`. On a developer host that target is
            # a cross target and rust-lld is the right answer. Inside this
            # Alpine builder the same triple *is* the host, so inheriting the
            # file makes every proc-macro -- `serde_derive`, `tokio-macros` --
            # link its host `.so` with rust-lld and fail on `-lgcc_s`/`-lc`.
            # Checked-in Cargo configuration is developer-host configuration;
            # the container owns its own toolchain settings and passes them as
            # environment, which is why the cross linker is set explicitly
            # below rather than read from the tree.
            #
            # Widening this glob therefore breaks the build. Verified, not
            # assumed: `/src/.[!.]*` fails at `tokio-macros`.
            'for f in /src/*; do b=$(basename "$f"); [ "$b" != target ] && [ "$b" != crates ] && ln -s "$f" /build/; done && '
            f"cp -r /src/crates /build/crates && "
            f"cargo build --locked --offline --release --target {rust_target} "
            "-p capsem-agent -p capsem-bench && "
            f"rm -f {rm_cmds} && "
            f"{cp_cmds} && chmod 555 {chmod_cmds} && {file_cmds}",
        ]
    )

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
    build: BuildConfig,
    arch_name: str,
    repo_root: Path,
    output_dir: Path,
) -> list[Path]:
    """Compile every guest architecture in the same locked offline helper.

    The helper always runs on the host CPU. A foreign guest target is reached
    with the target and exact clang package materialized into that host-platform
    image (the pinned Rust toolchain supplies rust-lld), replacing the former
    1194.7-second QEMU compile with an 89-second cross compile while preserving
    one sealed path for native and foreign targets.
    """
    return container_compile_agent(build, arch_name, repo_root, output_dir)


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
            lines.append(f"echo \"{key}=$({cmd} || echo 'N/A')\";")

    # -- Python packages --
    if "python" in config.package_sets:
        py_cmds = config.package_sets["python"].version_commands
        if py_cmds:
            lines.append('echo "# Python";')
            for key, cmd in py_cmds.items():
                lines.append(f"echo \"{key}=$({cmd} || echo 'N/A')\";")

    return "\n".join(lines)


def _validate_tool_versions(
    content: str,
    config: GuestImageConfig,
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
    output = _container_output(
        runtime,
        image_tag,
        platform,
        version_script,
        probe="tool versions",
        shell_option="-c",
    )
    versions_path = output_dir / "tool-versions.txt"
    versions_path.write_text(output)
    if validate:
        _validate_tool_versions(output, config)


def _container_output(
    runtime: str,
    image_tag: str,
    platform: str,
    command: str,
    *,
    probe: str,
    shell_option: str = "-lc",
) -> str:
    safe_probe = re.sub(r"[^a-z0-9]+", "-", probe.lower()).strip("-") or "command"
    container_name = f"capsem-probe-{safe_probe}-{uuid.uuid4().hex[:12]}"
    run_command = [
        runtime,
        "run",
        "--rm",
        "--pull",
        "never",
        "--network",
        "none",
        "--name",
        container_name,
        "--platform",
        platform,
        image_tag,
        "bash",
        shell_option,
        command,
    ]
    try:
        result = run_cmd(
            run_command,
            capture=True,
            timeout=CONTAINER_PROBE_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        cleanup_error: Exception | None = None
        try:
            run_cmd(
                [runtime, "rm", "-f", container_name],
                capture=True,
                echo=False,
                timeout=CONTAINER_PROBE_CLEANUP_TIMEOUT_SECONDS,
            )
        except Exception as caught:
            cleanup_error = caught
        detail = (
            f"{probe} container probe timed out after "
            f"{CONTAINER_PROBE_TIMEOUT_SECONDS}s "
            f"(image={image_tag}, platform={platform}, container={container_name})"
        )
        if cleanup_error is not None:
            detail += f"; forced cleanup failed: {cleanup_error}"
        raise RuntimeError(detail) from error
    return result.stdout


def extract_software_inventory(
    runtime: str,
    image_tag: str,
    platform: str,
    arch_name: str,
    output_dir: Path,
) -> Path:
    """Write installed package inventory captured from the built rootfs image."""
    from .manifest import collect_bom

    dpkg_output = _container_output(
        runtime,
        image_tag,
        platform,
        "dpkg-query -W -f='${Package}\\t${Version}\\t${Architecture}\\n'",
        probe="dpkg inventory",
    )
    pip_output = _container_output(
        runtime,
        image_tag,
        platform,
        "python3 -m pip list --format json",
        probe="Python inventory",
    )
    npm_output = _container_output(
        runtime,
        image_tag,
        platform,
        "npm ls --json --depth=0 --prefix /opt/ai-clis",
        probe="npm inventory",
    )
    manifest = collect_bom(
        arch=arch_name,
        dpkg_output=dpkg_output,
        pip_output=pip_output,
        npm_output=npm_output,
    )
    rows = [
        {
            "name": package.name,
            "version": package.version,
            "source": package.source,
            "architecture": package.arch or "all",
        }
        for package in manifest.packages
    ]
    rows.sort(key=lambda row: (row["source"], row["name"], row["architecture"], row["version"]))
    inventory = {
        "schema": "capsem.profile_software_inventory.v1",
        "architecture": arch_name,
        "packages": rows,
    }
    path = output_dir / SOFTWARE_INVENTORY_ASSET
    path.write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n")
    return path


def _cdxgen_command() -> list[str]:
    """Command installed and digest-verified in the asset tools helper."""
    return ["cdxgen"]


def _cdx_validate_command() -> list[str]:
    """Validator installed beside cdxgen from the same upstream release."""
    return ["cdx-validate"]


def _scanner_output_command(command: list[str], *, output_path: str) -> list[str]:
    """Run a root scanner, then return its bind-mounted output to the host."""
    return [
        "sh",
        "-eu",
        "-c",
        'output=$1; uid=$2; gid=$3; shift 3; "$@"; chown "$uid:$gid" "$output"',
        "capsem-scanner-output",
        output_path,
        str(os.getuid()),
        str(os.getgid()),
        *command,
    ]


def _normalize_cyclonedx_obom(
    path: Path,
    rootfs_dir: Path,
    *,
    architecture: str,
) -> None:
    """Remove build-host context while preserving exported-rootfs evidence."""
    document = json.loads(path.read_text())
    rootfs_prefixes = [str(rootfs_dir), "/rootfs"]
    relative_rootfs = os.path.relpath(rootfs_dir)
    if relative_rootfs not in rootfs_prefixes:
        rootfs_prefixes.append(relative_rootfs)

    def normalize(value: Any) -> Any:
        if isinstance(value, str):
            normalized = value
            for rootfs_prefix in rootfs_prefixes:
                normalized = normalized.replace(rootfs_prefix, "")
            return normalized or "/"
        if isinstance(value, list):
            normalized_items = [normalize(item) for item in value]
            return sorted(
                normalized_items,
                key=lambda item: json.dumps(item, sort_keys=True, separators=(",", ":")),
            )
        if isinstance(value, dict):
            normalized = {key: normalize(item) for key, item in value.items()}
            license_value = normalized.get("license")
            if isinstance(license_value, dict) and license_value.get("id") == "sendmail":
                # cdxgen 12.7.0 emits Debian's lowercase spelling, while the
                # SPDX identifier embedded in the CycloneDX schema is Sendmail.
                license_value["id"] = "Sendmail"
            return normalized
        return value

    document = normalize(document)
    document.pop("serialNumber", None)
    document.pop("annotations", None)
    metadata = document.setdefault("metadata", {})
    if not isinstance(metadata, dict):
        raise RuntimeError(f"OBOM {path} has non-object metadata")
    metadata.pop("timestamp", None)
    metadata["component"] = {
        "type": "operating-system",
        "name": f"capsem-rootfs-{architecture}",
        "version": "guest-rootfs",
        "properties": [
            {"name": "capsem:evidence:scope", "value": "exported-rootfs"},
            {"name": "capsem:guest:architecture", "value": architecture},
        ],
    }

    components = document.get("components")
    removed_refs: set[str] = set()
    if isinstance(components, list):
        retained_components = []
        for component in components:
            # trustinspector 2.5.1 collapses different CA certificates onto
            # identical bom-refs and then retains a nondeterministic winner.
            # Publishing that as evidence is misleading and changes the asset
            # digest between identical scans, so this OS OBOM excludes that
            # broken CBOM subset until the pinned scanner fixes the collision.
            if isinstance(component, dict) and component.get("type") == "cryptographic-asset":
                bom_ref = component.get("bom-ref")
                if isinstance(bom_ref, str):
                    removed_refs.add(bom_ref)
                continue
            retained_components.append(component)
        document["components"] = retained_components

    dependencies = document.get("dependencies")
    if isinstance(dependencies, list) and removed_refs:
        retained_dependencies = []
        for dependency in dependencies:
            if not isinstance(dependency, dict) or dependency.get("ref") in removed_refs:
                continue
            depends_on = dependency.get("dependsOn")
            if isinstance(depends_on, list):
                dependency["dependsOn"] = [ref for ref in depends_on if ref not in removed_refs]
            retained_dependencies.append(dependency)
        document["dependencies"] = retained_dependencies

    tools = metadata.get("tools")
    tool_components = tools.get("components") if isinstance(tools, dict) else None
    if isinstance(tool_components, list):
        for tool in tool_components:
            if not isinstance(tool, dict):
                continue
            # These fields describe the scanner executable on the build host,
            # not the exported guest rootfs, and differ across Linux/macOS.
            tool.pop("hashes", None)
            properties = tool.get("properties")
            if isinstance(properties, list):
                tool["properties"] = [
                    prop
                    for prop in properties
                    if not (isinstance(prop, dict) and prop.get("name") == "internal:binary_path")
                ]

    path.write_text(json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n")


_validate_cyclonedx_obom = validate_exported_rootfs_obom


def generate_cyclonedx_obom(
    rootfs_tar: Path,
    output_path: Path,
    *,
    repo_root: Path,
    architecture: str,
    runtime: str,
    tool_image: str,
    tool_platform: str,
    runtime_network: ContainerNetwork,
) -> Path:
    """Generate a CycloneDX OS OBOM for the exported rootfs tar.

    The build ledger records declared build inputs. This OBOM is the runtime
    inventory for what actually ended up in the base image.
    """
    import tempfile

    network_value = require_container_network(runtime_network)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    tmp_parent = repo_root / "cache" / "target" / "tmp"
    tmp_parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="capsem-obom-", dir=tmp_parent) as tmp:
        rootfs_dir = Path(tmp) / "rootfs"
        rootfs_dir.mkdir()
        run_cmd(
            [
                "tar",
                "--exclude=dev/*",
                "--exclude=proc/*",
                "--exclude=sys/*",
                "-xf",
                str(rootfs_tar),
                "-C",
                str(rootfs_dir),
            ],
            timeout=OBOM_COMMAND_TIMEOUT_SECONDS,
        )
        # cdxgen's rootfs mode is the only offline mode that inventories the
        # extracted guest. Its internal validation rejects Debian's lowercase
        # `sendmail` spelling before writing output, so emit first, normalize
        # that known SPDX spelling, then run the paired strict schema validator.
        run_cmd(
            [
                runtime,
                "run",
                "--rm",
                "--pull",
                "never",
                "--network",
                network_value,
                "--platform",
                tool_platform,
                "-v",
                f"{rootfs_dir}:/rootfs:ro",
                "-v",
                f"{output_path.parent.resolve()}:/output",
                tool_image,
                *_scanner_output_command(
                    [
                        *_cdxgen_command(),
                        "/rootfs",
                        "-t",
                        "rootfs",
                        "--no-validate",
                        "-o",
                        f"/output/{output_path.name}",
                    ],
                    output_path=f"/output/{output_path.name}",
                ),
            ],
            capture=True,
            timeout=OBOM_COMMAND_TIMEOUT_SECONDS,
        )
        _normalize_cyclonedx_obom(output_path, rootfs_dir, architecture=architecture)
    _validate_cyclonedx_obom(output_path, architecture=architecture)
    run_cmd(
        [
            runtime,
            "run",
            "--rm",
            "--pull",
            "never",
            "--network",
            network_value,
            "--platform",
            tool_platform,
            "-v",
            f"{output_path.parent.resolve()}:/output:ro",
            tool_image,
            *_cdx_validate_command(),
            "--strict",
            "--no-deep",
            "--fail-severity",
            "critical",
            "--no-include-manual",
            "-i",
            f"/output/{output_path.name}",
        ],
        capture=True,
        timeout=OBOM_COMMAND_TIMEOUT_SECONDS,
    )
    return output_path


def _blake3_hex(path: Path) -> str:
    """Compute BLAKE3 hash of a file, returning the hex digest."""
    import blake3

    hasher = blake3.blake3()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            hasher.update(chunk)
    return hasher.hexdigest()


def _asset_digest_hex(path: Path) -> tuple[str, str]:
    """Stream one asset once and return its BLAKE3 and SHA-256 digests."""
    import blake3

    blake3_hasher = blake3.blake3()
    sha256_hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            blake3_hasher.update(chunk)
            sha256_hasher.update(chunk)
    return blake3_hasher.hexdigest(), sha256_hasher.hexdigest()


def _utc_now_iso() -> str:
    return datetime.datetime.now(datetime.UTC).isoformat().replace("+00:00", "Z")


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
            "dependency_artifacts": config.build.asset_dependencies.architectures[
                arch_name
            ].model_dump(mode="json"),
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


def asset_min_binary(binary_version: str) -> str:
    """Lowest binary these assets support: the base of the binary's release line.

    Derived, never hardcoded. A literal floor sat here across a change of
    release line and put every binary *below* the minimum its own assets
    declared, so installation failed with "no compatible asset release" even
    though the asset release was present and otherwise valid.

    The line base rather than the exact version, so a compatibility window
    survives: any binary sharing this MAJOR.MINOR runs these assets, and a
    patch release does not force everyone to re-hydrate.
    """
    major, minor, *_ = binary_version.split(".")
    return f"{major}.{minor}.0"


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
    if current in releases and _asset_identity(
        releases[current].get("arches", {})
    ) == _asset_identity(arch_assets):
        return current
    for version in releases:
        if not version.startswith(f"{date_prefix}."):
            continue
        try:
            patch = max(patch, int(version.rsplit(".", 1)[1]) + 1)
        except ValueError:
            continue
    return f"{date_prefix}.{patch}"


def _asset_identity(arches: dict[str, dict[str, dict]]) -> dict[str, dict[str, dict]]:
    """Return the version-owning identity, excluding enrichable digest metadata."""
    return {
        arch: {
            logical_name: {
                "hash": entry.get("hash"),
                "size": entry.get("size"),
            }
            for logical_name, entry in assets.items()
        }
        for arch, assets in arches.items()
    }


def _hash_filename(logical_name: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical_name:
        stem, ext = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{ext}"
    return f"{logical_name}-{prefix}"


def _restore_canonical_assets_from_existing_manifest(output_dir: Path) -> None:
    manifest_path = output_dir / "manifest.json"
    if not manifest_path.is_file():
        return
    try:
        manifest = json.loads(manifest_path.read_text())
    except json.JSONDecodeError:
        return
    for release in manifest.get("assets", {}).get("releases", {}).values():
        for arch_name, assets in release.get("arches", {}).items():
            arch_dir = output_dir / arch_name
            if not arch_dir.is_dir():
                continue
            for logical_name, meta in assets.items():
                canonical = arch_dir / logical_name
                if canonical.exists():
                    continue
                digest = meta.get("hash")
                if not isinstance(digest, str):
                    continue
                alias = arch_dir / _hash_filename(logical_name, digest)
                if not alias.is_file():
                    continue
                # Through the audited chokepoint. Both sides here are build
                # output, so this still hardlinks -- but "happens to be safe"
                # is not a guarantee, and `stage` is the thing that checks
                # rather than the comment that claims.
                auditfs.stage(alias, canonical)


def generate_checksums(output_dir: Path, version: str) -> Path:
    """Generate BLAKE3 checksums and manifest.json for all assets."""
    _restore_canonical_assets_from_existing_manifest(output_dir)

    # Collect all asset files across arch subdirs
    arch_dirs = [d for d in output_dir.iterdir() if d.is_dir() and d.name != "current"]
    all_files: list[str] = []
    for arch_dir in sorted(arch_dirs):
        arch_name = arch_dir.name
        rootfs_name = _select_rootfs_asset(arch_dir)
        arch_has_assets = (
            rootfs_name is not None
            or (arch_dir / OBOM_ASSET).is_file()
            or any((arch_dir / filename).is_file() for filename in BOOT_ASSETS)
        )
        if arch_has_assets:
            for filename in BOOT_ASSETS:
                if not (arch_dir / filename).is_file():
                    raise FileNotFoundError(f"{arch_dir / filename}")
            if rootfs_name is None:
                raise FileNotFoundError(f"{arch_dir / 'rootfs.erofs'}")
        for filename in BOOT_ASSETS:
            if (arch_dir / filename).is_file():
                all_files.append(f"{arch_name}/{filename}")
        if rootfs_name:
            all_files.append(f"{arch_name}/{rootfs_name}")
        if (arch_dir / OBOM_ASSET).is_file():
            all_files.append(f"{arch_name}/{OBOM_ASSET}")
        if (arch_dir / SOFTWARE_INVENTORY_ASSET).is_file():
            all_files.append(f"{arch_name}/{SOFTWARE_INVENTORY_ASSET}")

    if not all_files:
        # Flat layout fallback
        flat_rootfs_name = _select_rootfs_asset(output_dir)
        flat_has_assets = (
            flat_rootfs_name is not None
            or (output_dir / OBOM_ASSET).is_file()
            or any((output_dir / f).is_file() for f in BOOT_ASSETS)
        )
        if flat_has_assets:
            for filename in BOOT_ASSETS:
                if not (output_dir / filename).is_file():
                    raise FileNotFoundError(f"{output_dir / filename}")
            if flat_rootfs_name is None:
                raise FileNotFoundError(f"{output_dir / 'rootfs.erofs'}")
        for f in BOOT_ASSETS:
            if (output_dir / f).is_file():
                all_files.append(f)
        if flat_rootfs_name:
            rootfs_name = flat_rootfs_name
            all_files.append(rootfs_name)
        if (output_dir / OBOM_ASSET).is_file():
            all_files.append(OBOM_ASSET)
        if (output_dir / SOFTWARE_INVENTORY_ASSET).is_file():
            all_files.append(SOFTWARE_INVENTORY_ASSET)

    # Compute both public digests in one streaming pass. Channel assembly owns
    # graph rendering, not re-hashing immutable multi-gigabyte build outputs.
    b3sums_lines = []
    digests: dict[str, tuple[str, str]] = {}
    for filepath in all_files:
        full_path = output_dir / filepath
        b3hash, sha256 = _asset_digest_hex(full_path)
        digests[filepath] = (b3hash, sha256)
        b3sums_lines.append(f"{b3hash}  {filepath}")
    (output_dir / "B3SUMS").write_text("\n".join(b3sums_lines) + "\n")

    arch_assets: dict[str, dict[str, dict]] = {}
    for filepath in all_files:
        full_path = output_dir / filepath
        b3hash, sha256 = digests[filepath]
        size = full_path.stat().st_size

        if "/" in filepath:
            arch_name, filename = filepath.split("/", 1)
        else:
            arch_name = "unknown"
            filename = filepath

        arch_assets.setdefault(arch_name, {})[filename] = {
            "hash": b3hash,
            "sha256": sha256,
            "size": size,
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
                    "min_binary": asset_min_binary(version),
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

    # Create cache/target/assets/current symlink pointing to the most recently built arch.
    # Tauri bundle resources reference cache/target/assets/current/ so they resolve on any platform.
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
) -> Path:
    """Write rendered Dockerfile and copy required files into a build context."""
    guest_dir = Path(config.guest_dir_path) if config.guest_dir_path else repo_root / "guest"
    # Render Dockerfile
    dockerfile_content = render_dockerfile(template_name, config, arch_name)
    dockerfile_path = context_dir / "Dockerfile"
    dockerfile_path.write_text(dockerfile_content)

    if template_name == config.build.asset_dependencies.rootfs_template:
        packages_dir = guest_dir / "config" / "packages"
        if "python" in config.package_sets:
            python_lock = packages_dir / "python-requirements.lock"
            if not python_lock.is_file():
                raise FileNotFoundError(python_lock)
            shutil.copy2(str(python_lock), str(context_dir / python_lock.name))
        if "npm" in config.package_sets:
            for name in ("npm-package.json", "npm-package-lock.json"):
                source = packages_dir / name
                if not source.is_file():
                    raise FileNotFoundError(source)
                shutil.copy2(str(source), str(context_dir / name))
        if config.profile_build_script:
            if not config.profile_build_script_path:
                raise FileNotFoundError("profile_build_script_path")
            profile_build = Path(config.profile_build_script_path)
            if not profile_build.is_file():
                raise FileNotFoundError(profile_build)
            shutil.copy2(str(profile_build), str(context_dir / "profile-build.sh"))
    elif template_name == "Dockerfile.rootfs.j2":
        # CA cert
        shutil.copy2(
            str(
                repo_root
                / "crates"
                / "capsem-core"
                / "resources"
                / "ca"
                / "capsem-ca.crt"
            ),
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
        # Agent binaries (if they exist in context already from cross_compile_agent)
        # They may have been copied to context_dir by the pipeline before this call

    elif template_name == "Dockerfile.kernel.j2":
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
    repo_root: Path | None = None,
) -> None:
    """Build a Docker image for the given architecture.

    Full pipeline for one arch+template. Outputs to output_dir/{arch_name}/.
    """
    import tempfile

    if repo_root is None:
        repo_root = Path.cwd()
    if output_dir is None:
        output_dir = repo_root / "cache" / "target" / "assets"

    arch = config.build.architectures[arch_name]
    runtime = detect_runtime()
    # Sync container VM clock with host to prevent apt date errors
    sync_container_clock()

    # Per-arch output directory
    arch_output = output_dir / arch_name
    arch_output.mkdir(parents=True, exist_ok=True)

    template_name = f"Dockerfile.{template}.j2"
    tag = f"capsem-{template}-{arch_name}"

    # Use a temporary directory inside the project root's cache/target/ folder.
    # On macOS, system temp dirs (/var/folders) are often not mountable by Docker/Colima.
    build_tmp = repo_root / "cache" / "target" / "tmp"
    build_tmp.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix=f"capsem-build-{template}-", dir=build_tmp) as tmpdir:
        context_dir = Path(tmpdir)
        dependency_image = require_asset_dependencies(
            runtime,
            config,
            arch_name,
            template,
        )

        if template == "kernel":
            kernel_version = config.build.kernel.version
            print(f"Kernel: {kernel_version}")

            dockerfile_path = prepare_build_context(
                config,
                arch_name,
                template_name,
                context_dir,
                repo_root,
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
            build_inputs["dependency_image"] = dependency_image.as_record()
            docker_build(
                runtime,
                tag,
                context_dir / "Dockerfile",
                context_dir,
                arch.docker_platform,
                network=config.build.asset_dependencies.source_build_network,
                build_args={"BASE": dependency_image.reference},
                ci_cache=False,
            )
            if require_asset_dependencies(runtime, config, arch_name, template) != dependency_image:
                raise RuntimeError("kernel dependency image moved during sealed source build")
            vmlinuz, initrd = extract_kernel_assets(
                runtime,
                tag,
                arch.docker_platform,
                arch_output,
            )
            remove_image(runtime, tag)
            _append_build_ledger(
                arch_output,
                {
                    "stage": "kernel.assets",
                    "inputs": build_inputs,
                    "kernel_version": kernel_version,
                    "kernel_sha256": config.build.kernel.sha256,
                    "outputs": [
                        _file_ledger_entry(vmlinuz, base=arch_output),
                        _file_ledger_entry(initrd, base=arch_output),
                    ],
                },
            )
            print(f"  vmlinuz:    {vmlinuz}")
            print(f"  initrd.img: {initrd}")

        elif template == "rootfs":
            # Cross-compile agent binaries
            print(f"Cross-compiling guest binaries for {arch.rust_target}...")
            binaries = cross_compile_agent(
                config.build,
                arch_name,
                repo_root,
                context_dir,
            )
            for b in binaries:
                print(f"  {b.name}: {b.stat().st_size} bytes")

            dockerfile_path = prepare_build_context(
                config,
                arch_name,
                template_name,
                context_dir,
                repo_root,
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
            build_inputs["dependency_image"] = dependency_image.as_record()
            _append_build_ledger(
                arch_output,
                _rootfs_config_input_record(config, arch_name),
            )
            docker_build(
                runtime,
                tag,
                context_dir / "Dockerfile",
                context_dir,
                arch.docker_platform,
                network=config.build.asset_dependencies.source_build_network,
                build_args={"BASE": dependency_image.reference},
                ci_cache=False,
            )
            if require_asset_dependencies(runtime, config, arch_name, template) != dependency_image:
                raise RuntimeError("rootfs dependency image moved during sealed source build")

            print("Extracting installed software inventory...")
            software_inventory_path = extract_software_inventory(
                runtime,
                tag,
                arch.docker_platform,
                arch_name,
                arch_output,
            )
            _append_build_ledger(
                arch_output,
                {
                    "stage": "rootfs.software_inventory",
                    "inputs": build_inputs,
                    "outputs": [_file_ledger_entry(software_inventory_path, base=arch_output)],
                },
            )

            # Export and compress
            tar_path = arch_output / "rootfs.tar"
            print("Exporting rootfs filesystem...")
            export_container_fs(runtime, tag, arch.docker_platform, tar_path)
            validate_rootfs_export(tar_path, config.build.rootfs)
            tar_entry = _file_ledger_entry(tar_path, base=arch_output)
            _append_build_ledger(
                arch_output,
                {
                    "stage": "rootfs.export",
                    "inputs": build_inputs,
                    "intermediates": [tar_entry],
                },
            )

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
                runtime,
                tar_path,
                erofs_path,
                erofs_compression,
                erofs_cluster_size,
                erofs_level,
                tool_image=_asset_tools_image(config, repo_root),
                runtime_network=config.build.asset_tools.runtime_network,
            )
            validate_erofs_size(erofs_path, config.build.rootfs)
            erofs_entry = _file_ledger_entry(erofs_path, base=arch_output)
            _append_build_ledger(
                arch_output,
                {
                    "stage": "rootfs.erofs",
                    "inputs": build_inputs,
                    "intermediates": [tar_entry],
                    "erofs": {
                        "compression": erofs_compression,
                        "compression_level": erofs_level,
                        "cluster_size": erofs_cluster_size,
                        "utils_image": _asset_tools_image(config, repo_root),
                    },
                    "outputs": [erofs_entry],
                },
            )
            print("Generating CycloneDX OBOM...")
            obom_path = arch_output / OBOM_ASSET
            generate_cyclonedx_obom(
                tar_path,
                obom_path,
                repo_root=repo_root,
                architecture=arch_name,
                runtime=runtime,
                tool_image=_asset_tools_image(config, repo_root),
                tool_platform=_native_linux_platform(),
                runtime_network=config.build.asset_tools.runtime_network,
            )
            obom_entry = _file_ledger_entry(obom_path, base=arch_output)
            _append_build_ledger(
                arch_output,
                {
                    "stage": "rootfs.obom",
                    "inputs": build_inputs,
                    "intermediates": [tar_entry],
                    "generator": "cdxgen",
                    "outputs": [obom_entry],
                },
            )
            tar_path.unlink(missing_ok=True)

            print("Extracting tool versions...")
            extract_tool_versions(runtime, tag, arch.docker_platform, arch_output, config)
            versions_path = arch_output / "tool-versions.txt"
            if versions_path.is_file():
                _append_build_ledger(
                    arch_output,
                    {
                        "stage": "rootfs.tool_versions",
                        "inputs": build_inputs,
                        "outputs": [_file_ledger_entry(versions_path, base=arch_output)],
                    },
                )
            remove_image(runtime, tag)

            print(f"  rootfs.erofs:    {erofs_path}")


def build_all_architectures(
    config: GuestImageConfig,
    *,
    template: str = "rootfs",
    output_dir: Path | None = None,
    repo_root: Path | None = None,
) -> None:
    """Build Docker images for all configured architectures."""
    if repo_root is None:
        repo_root = Path.cwd()
    if output_dir is None:
        output_dir = repo_root / "cache" / "target" / "assets"

    for arch_name in config.build.architectures:
        print(f"\n=== Building {template} for {arch_name} ===")
        build_image(
            config,
            arch_name,
            template=template,
            output_dir=output_dir,
            repo_root=repo_root,
        )

    # Prune dangling images left by multi-stage builds
    runtime = detect_runtime()
    try:
        run_cmd([runtime, "image", "prune", "-f"], capture=True)
        print("Pruned dangling images.")
    except RuntimeError:
        pass

    if template != "kernel":
        version = get_project_version(repo_root)
        print(f"\nGenerating checksums (version {version})...")
        generate_checksums(output_dir, version)

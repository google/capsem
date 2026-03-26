"""Dockerfile generation from GuestImageConfig via Jinja2 templates.

Renders Dockerfiles for rootfs and kernel builds using config-driven
Jinja2 templates. Supports multi-architecture output (arm64, x86_64).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from jinja2 import Environment, FileSystemLoader

from capsem.builder.models import GuestImageConfig, PackageManager

TEMPLATES_DIR = Path(__file__).parent / "templates"

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


def build_image(
    config: GuestImageConfig,
    arch_name: str,
    **kwargs: Any,
) -> None:
    """Build a Docker image for the given architecture.

    Not yet implemented -- actual Docker execution is Phase 6 (CLI).
    """
    raise NotImplementedError("Docker image building will be implemented in Phase 6")


def build_all_architectures(
    config: GuestImageConfig,
    **kwargs: Any,
) -> None:
    """Build Docker images for all configured architectures.

    Not yet implemented -- actual Docker execution is Phase 6 (CLI).
    """
    raise NotImplementedError("Docker image building will be implemented in Phase 6")

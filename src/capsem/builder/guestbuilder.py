"""Where one architecture's guest binaries compile, and the key for that image.

A foreign target used to be built by running the target's own platform child
under QEMU. Measured cold on a 16-core Linux host, the six aarch64 guest
binaries took 1194.7s that way against 86s cross-compiled from the amd64 base,
and a profile release run compiles that graph three times.

So the builder is chosen by the *host*, not by the target: the image is always
the host platform's exact Rust child, and a foreign target is reached by adding
the target and a C toolchain at image-build time. That is symmetric, which is
the point -- on Apple Silicon the emulated lane is the x86_64 one, and the same
resolution fixes it without a second mechanism.
"""

from __future__ import annotations

import hashlib
import platform
from dataclasses import dataclass
from pathlib import Path

from capsem.builder.models import BuildConfig

# Every spelling `platform.machine()` returns for the two supported CPUs.
_HOST_ALIASES = {
    "aarch64": "arm64",
    "arm64": "arm64",
    "x86_64": "x86_64",
    "amd64": "x86_64",
}


@dataclass(frozen=True)
class BuilderEnvironment:
    """The image one architecture's guest binaries actually compile in."""

    base_image: str
    """The exact platform child the builder is built FROM: always the host's."""

    docker_platform: str
    """The platform the builder image is built and run on: always the host's."""

    rust_target: str
    """The target being produced, which may be foreign to `docker_platform`."""

    cross: bool
    """Whether the target is foreign, and the image must materialize it."""

    cross_packages: tuple[str, ...]
    """Exact APK package specs materialized only for a foreign target."""


def host_architecture(build: BuildConfig) -> str:
    """The configured architecture matching the CPU running this build.

    On macOS this is still the Linux container platform's architecture: Docker
    runs the builder inside a Linux VM on the same CPU, so an Apple Silicon
    host builds `linux/arm64` natively and reaches `x86_64` by crossing.
    """
    machine = platform.machine().lower()
    name = _HOST_ALIASES.get(machine)
    if name is None or name not in build.architectures:
        raise ValueError(f"no guest builder architecture for host CPU: {machine}")
    return name


def environment(build: BuildConfig, arch_name: str) -> BuilderEnvironment:
    """Resolve where `arch_name`'s guest binaries compile."""
    try:
        target = build.architectures[arch_name]
    except KeyError:
        raise ValueError(f"unknown guest Rust builder architecture: {arch_name}") from None

    host_name = host_architecture(build)
    source = build.architectures[host_name]
    return BuilderEnvironment(
        base_image=source.rust_builder_base_image,
        docker_platform=source.docker_platform,
        rust_target=target.rust_target,
        cross=arch_name != host_name,
        cross_packages=build.guest_rust_builder.cross_packages if arch_name != host_name else (),
    )


def _identity_file(root: Path, relative: str) -> bytes:
    path = root / relative
    if not path.is_file():
        raise ValueError(f"guest Rust builder identity input is missing: {relative}")
    return path.read_bytes()


def image_tag(build: BuildConfig, arch_name: str, root: Path) -> str:
    """Return the local cache tag selected by every declared build input.

    This is deliberately not called an OCI content address: Cargo's fetched
    registry metadata and Docker layer metadata can differ between cold
    materializations even though Cargo.lock fixes every package byte consumed
    by the sealed runtime build.
    """
    resolved = environment(build, arch_name)

    settings = build.guest_rust_builder
    digest = hashlib.blake2b(digest_size=16)
    # Keyed by the environment actually used, not by the target's own platform
    # child. The same target reached from two different hosts is two different
    # images, and a native helper and a cross one must never share a tag.
    for value in (
        arch_name,
        resolved.docker_platform,
        resolved.rust_target,
        resolved.base_image,
        "cross" if resolved.cross else "native",
        *resolved.cross_packages,
    ):
        digest.update(value.encode("utf-8"))
        digest.update(b"\0")
    for relative in (settings.dockerfile, *settings.identity_inputs):
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(_identity_file(root, relative))
        digest.update(b"\0")
    return settings.tag_template.format(arch=arch_name, digest=digest.hexdigest())


def image_repository(build: BuildConfig, arch_name: str) -> str:
    """Repository whose tags are generations of one architecture's helper."""
    rendered = build.guest_rust_builder.tag_template.format(
        arch=arch_name,
        digest="identity",
    )
    return rendered.rsplit(":", 1)[0]


def build_arguments(resolved: BuilderEnvironment) -> list[str]:
    """The explicit inputs the shared builder Dockerfile receives.

    `CROSS` decides whether the image materializes the target and a C toolchain
    or asserts the base already carries the target. It is an explicit argument
    rather than something the Dockerfile infers, so the two shapes are visible
    in the recorded `docker build` argv.
    """
    return [
        f"BASE={resolved.base_image}",
        f"RUST_TARGET={resolved.rust_target}",
        f"CROSS={'1' if resolved.cross else '0'}",
        f"CROSS_PACKAGES={' '.join(resolved.cross_packages)}",
    ]

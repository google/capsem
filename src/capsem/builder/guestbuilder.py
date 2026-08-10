"""Input cache key for the pre-materialized guest Rust cross-build environment."""

from __future__ import annotations

import hashlib
from pathlib import Path

from capsem.builder.models import ArchConfig, BuildConfig


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
    try:
        arch = build.architectures[arch_name]
    except KeyError:
        raise ValueError(f"unknown guest Rust builder architecture: {arch_name}") from None

    settings = build.guest_rust_builder
    digest = hashlib.blake2b(digest_size=16)
    for value in (
        arch_name,
        arch.docker_platform,
        arch.rust_target,
        arch.rust_builder_base_image,
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


def build_arguments(arch: ArchConfig) -> list[str]:
    """The two inputs the shared builder Dockerfile receives."""
    return [
        f"BASE={arch.rust_builder_base_image}",
        f"RUST_TARGET={arch.rust_target}",
    ]

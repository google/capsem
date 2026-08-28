"""Input identity for the offline EROFS and CycloneDX helper image."""

from __future__ import annotations

import hashlib
from pathlib import Path

from .models import ArchConfig, BuildConfig

INPUT_KEY_LABEL = "org.capsem.asset-tools.input-key"


def image_tag(build: BuildConfig, arch_name: str, root: Path) -> str:
    """Derive the helper tag from every executable materialization input."""
    try:
        arch = build.architectures[arch_name]
        downloads = build.asset_tools.architectures[arch_name]
    except KeyError:
        raise ValueError(f"unknown asset tool architecture: {arch_name}") from None
    settings = build.asset_tools
    dockerfile = root / settings.dockerfile
    if not dockerfile.is_file():
        raise ValueError(f"asset tools Dockerfile is missing: {settings.dockerfile}")
    digest = hashlib.blake2b(digest_size=16)
    for value in (
        arch_name,
        arch.docker_platform,
        arch.base_image,
        arch.rust_builder_base_image,
        settings.debian_snapshot_base,
        settings.debian_security_snapshot_base,
        settings.debian_snapshot_id,
        settings.materialize_network,
        settings.runtime_network,
        downloads.cdxgen.url,
        downloads.cdxgen.sha256,
        downloads.cdx_validate.url,
        downloads.cdx_validate.sha256,
    ):
        digest.update(value.encode())
        digest.update(b"\0")
    digest.update(dockerfile.read_bytes())
    return settings.tag_template.format(arch=arch_name, digest=digest.hexdigest())


def image_repository(build: BuildConfig, arch_name: str) -> str:
    rendered = build.asset_tools.tag_template.format(arch=arch_name, digest="identity")
    return rendered.rsplit(":", 1)[0]


def build_arguments(build: BuildConfig, arch_name: str, identity: str) -> list[str]:
    arch: ArchConfig = build.architectures[arch_name]
    settings = build.asset_tools
    downloads = settings.architectures[arch_name]
    return [
        f"BASE={arch.base_image}",
        f"TRUSTSTORE_IMAGE={arch.rust_builder_base_image}",
        f"DEBIAN_SNAPSHOT_BASE={settings.debian_snapshot_base}",
        f"DEBIAN_SECURITY_SNAPSHOT_BASE={settings.debian_security_snapshot_base}",
        f"DEBIAN_SNAPSHOT_ID={settings.debian_snapshot_id}",
        f"CDXGEN_URL={downloads.cdxgen.url}",
        f"CDXGEN_SHA256={downloads.cdxgen.sha256}",
        f"CDX_VALIDATE_URL={downloads.cdx_validate.url}",
        f"CDX_VALIDATE_SHA256={downloads.cdx_validate.sha256}",
        f"INPUT_IDENTITY={identity}",
    ]

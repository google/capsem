"""Identity for network-open asset dependency materializers."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from pathlib import Path

from capsem.builder.models import GuestImageConfig

INPUT_KEY_LABEL = "org.capsem.asset-dependencies.input-key"


@dataclass(frozen=True)
class AssetDependencyImage:
    """One runnable local reference bound to its verified exact image ID."""

    reference: str
    image_id: str

    def as_record(self) -> dict[str, str]:
        return {"reference": self.reference, "image_id": self.image_id}


def image_tag(
    config: GuestImageConfig,
    arch_name: str,
    template: str,
    rendered: bytes,
) -> str:
    """Key a helper by every byte its dependency acquisition can execute."""
    settings = config.build.asset_dependencies
    digest = hashlib.blake2b(digest_size=16)
    for value in (
        arch_name,
        template,
        config.build.architectures[arch_name].docker_platform,
        config.build.architectures[arch_name].rust_builder_base_image,
        config.manifest.name if config.manifest else "unscoped",
        config.manifest.version if config.manifest else "unversioned",
    ):
        digest.update(value.encode())
        digest.update(b"\0")
    digest.update(rendered)
    digest.update(b"\0")
    if template == "rootfs" and config.profile_build_script:
        if config.profile_build_script_path is None:
            raise ValueError("profile build script is enabled without a path")
        path = Path(config.profile_build_script_path)
        if not path.is_file():
            raise ValueError(f"profile build script is missing: {path}")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return settings.tag_template.format(
        template=template,
        arch=arch_name,
        digest=digest.hexdigest(),
    )

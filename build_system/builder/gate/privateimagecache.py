"""Private GHCR generations for network-open build dependency images."""

from __future__ import annotations

import json
from pathlib import Path

from ..privateimages import SOURCE_LABEL, PrivateImage
from .config import GateConfig
from .docker import Docker
from .errors import GateError
from .proc import Runner


def _valid(docker: Docker, config: GateConfig, image: PrivateImage) -> bool:
    return (
        docker.image_label(image.local, image.identity_label) == image.identity
        and docker.image_label(image.local, SOURCE_LABEL) == config.private_images.source
    )


def try_import(runner: Runner, config: GateConfig, image: PrivateImage) -> bool:
    """Pull and validate an exact helper, returning false for a cold fallback."""
    remote = image.remote(config.private_images.registry)
    if not runner.succeeds(["docker", "pull", "--platform", image.platform, remote]):
        runner.note(f"private image generation unavailable; building locally: {remote}")
        return False
    runner.run(["docker", "tag", remote, image.local])
    docker = Docker(runner)
    valid = docker.image_id(image.local, platform=image.platform).startswith("sha256:") and _valid(
        docker, config, image
    )
    if not valid:
        runner.succeeds(["docker", "image", "rm", image.local, remote])
        runner.note(f"private image generation failed identity validation: {remote}")
        return False
    runner.succeeds(["docker", "image", "rm", remote])
    runner.note(f"reused private image generation: {remote}")
    return True


def try_import_tag(
    runner: Runner,
    config: GateConfig,
    *,
    tag: str,
    platform: str,
    identity_label: str,
) -> bool:
    """Import a helper whose tag is also its complete input identity."""
    return try_import(
        runner,
        config,
        PrivateImage(
            local=tag,
            platform=platform,
            generation=tag.rsplit(":", 1)[1],
            identity_label=identity_label,
            identity=tag,
        ),
    )


def publish(
    runner: Runner,
    config: GateConfig,
    images: tuple[PrivateImage, ...],
    manifest: Path,
) -> None:
    """Push absent immutable generations and record packages for retention."""
    docker = Docker(runner)
    records = []
    for image in images:
        if not docker.image_exists(image.local, platform=image.platform) or not _valid(
            docker, config, image
        ):
            raise GateError(f"private image is not the expected local generation: {image.local}")
        remote = image.remote(config.private_images.registry)
        if runner.succeeds(["docker", "manifest", "inspect", remote]):
            runner.note(f"private image generation is already published: {remote}")
        else:
            runner.run(["docker", "tag", image.local, remote])
            for attempt in range(1, config.private_images.publish_attempts + 1):
                if runner.succeeds(["docker", "push", remote]):
                    break
                runner.note(
                    "private image manifest publication failed "
                    f"(attempt {attempt}/{config.private_images.publish_attempts}): {remote}"
                )
            else:
                raise GateError(
                    "private image publication exhausted "
                    f"{config.private_images.publish_attempts} attempts: {remote}"
                )
            runner.succeeds(["docker", "image", "rm", remote])
        records.append(
            {
                "generation": image.generation,
                "package": image.package(config.private_images.registry),
                "platform": image.platform,
                "reference": remote,
            }
        )
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(
        json.dumps(
            {
                "keep_generations": config.private_images.keep_generations,
                "images": records,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

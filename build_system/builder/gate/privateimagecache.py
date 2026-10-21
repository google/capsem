"""Private GHCR generations for network-open build dependency images."""

from __future__ import annotations

import json
from pathlib import Path

from ..privateimages import SOURCE_LABEL, PrivateImage
from .config import GateConfig
from .docker import Docker
from .errors import GateError
from .fileactions import write_text
from .proc import Runner


def _valid(docker: Docker, config: GateConfig, image: PrivateImage) -> bool:
    return (
        docker.image_label(image.local, image.identity_label) == image.identity
        and docker.image_label(image.local, SOURCE_LABEL) == config.private_images.source
    )


def try_import(runner: Runner, config: GateConfig, image: PrivateImage) -> bool:
    """Pull and validate an exact helper, returning false for a cold fallback."""
    remote = image.remote(config.private_images.registry)
    docker = Docker(runner)
    if not docker.try_pull(remote, platform=image.platform):
        runner.note(f"private image generation unavailable; building locally: {remote}")
        return False
    docker.tag(remote, image.local)
    valid = docker.image_id(image.local, platform=image.platform).startswith("sha256:") and _valid(
        docker, config, image
    )
    if not valid:
        docker.remove_image(image.local, remote)
        runner.note(f"private image generation failed identity validation: {remote}")
        return False
    docker.remove_image(remote)
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


def try_import_previous(
    runner: Runner,
    config: GateConfig,
    *,
    repository: str,
    platform: str,
    identity_label: str,
    current_generation: str,
) -> str | None:
    """Import the prior published generation as a BuildKit cache source."""
    manifest = config.path(config.private_images.manifest)
    if not manifest.is_file():
        return None
    try:
        records = json.loads(manifest.read_text(encoding="utf-8"))["images"]
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise GateError(f"invalid private image manifest {manifest}: {error}") from error
    if not isinstance(records, list) or not all(isinstance(record, dict) for record in records):
        raise GateError(f"invalid private image records in {manifest}")
    package = PrivateImage(
        local=f"{repository}:{current_generation}",
        platform=platform,
        generation=current_generation,
        identity_label=identity_label,
        identity=f"{repository}:{current_generation}",
    ).package(config.private_images.registry)
    for record in reversed(records):
        if (
            record.get("package") != package
            or record.get("platform") != platform
            or record.get("generation") == current_generation
        ):
            continue
        generation = record.get("generation")
        if not isinstance(generation, str) or not generation:
            raise GateError(f"invalid private image generation in {manifest}")
        local = f"{repository}:{generation}"
        image = PrivateImage(
            local=local,
            platform=platform,
            generation=generation,
            identity_label=identity_label,
            identity=local,
        )
        if try_import(runner, config, image):
            runner.note(
                f"using prior private generation as build cache: {image.remote(config.private_images.registry)}"
            )
            return local
        return None
    return None


def publish(
    runner: Runner,
    config: GateConfig,
    images: tuple[PrivateImage, ...],
    manifest: Path,
) -> None:
    """Push absent immutable generations and record packages for retention."""
    # Validate the whole batch before even the first approved image can push.
    unexpected = {image.package(config.private_images.registry) for image in images} - set(
        config.private_images.approved_packages
    )
    if unexpected:
        raise GateError(
            "unapproved private image package (maintainer approval required): "
            + ", ".join(sorted(unexpected))
        )
    docker = Docker(runner)
    records = []
    for image in images:
        if not docker.image_exists(image.local, platform=image.platform) or not _valid(
            docker, config, image
        ):
            raise GateError(f"private image is not the expected local generation: {image.local}")
        remote = image.remote(config.private_images.registry)
        if docker.manifest_exists(remote):
            runner.note(f"private image generation is already published: {remote}")
        else:
            docker.tag(image.local, remote)
            for attempt in range(1, config.private_images.publish_attempts + 1):
                if docker.try_push(remote):
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
            docker.remove_image(remote)
        records.append(
            {
                "generation": image.generation,
                "package": image.package(config.private_images.registry),
                "platform": image.platform,
                "reference": remote,
            }
        )
    write_text(
        manifest,
        json.dumps(
            {
                "keep_generations": config.private_images.keep_generations,
                "images": records,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )

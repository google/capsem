"""Fail-closed identity checks shared by input-keyed Docker helpers."""

from __future__ import annotations

import re

from .docker import Docker
from .errors import GateError


def exact_image_id(
    docker: Docker,
    image: str,
    *,
    platform: str | None,
    subject: str,
) -> str:
    found = docker.image_id(image, platform=platform)
    if re.fullmatch(r"sha256:[0-9a-f]{64}", found) is None:
        raise GateError(f"{subject} {image} has invalid image ID {found!r}")
    return found


def exact_image_reference(
    docker: Docker,
    image: str,
    *,
    platform: str | None,
    expected_id: str,
    subject: str,
) -> str:
    reference = docker.build_reference(image)
    resolved = exact_image_id(
        docker,
        reference,
        platform=platform,
        subject=subject,
    )
    if resolved != expected_id:
        raise GateError(
            f"{subject} {image} moved while resolving its build reference: "
            f"expected {expected_id}, but {reference} resolves to {resolved}"
        )
    return reference


def require_exact_image(
    docker: Docker,
    image: str,
    *,
    platform: str | None,
    expected_id: str,
    subject: str,
) -> None:
    """Refuse if a build reference no longer resolves to its recorded image."""
    found = exact_image_id(docker, image, platform=platform, subject=subject)
    if found != expected_id:
        raise GateError(f"{subject} moved: expected {expected_id}, found {found}")


def require_input_key(
    docker: Docker,
    tag: str,
    *,
    label: str,
    subject: str,
) -> None:
    found = docker.image_label(tag, label)
    if found != tag:
        raise GateError(
            f"{subject} tag {tag} carries input key {found!r}; refusing a poisoned warm tag"
        )

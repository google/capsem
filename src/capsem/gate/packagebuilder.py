"""Materialize the network-open dependency helper for one Linux package."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from pathlib import Path

from .config import Arch, GateConfig
from .docker import Docker
from .errors import GateError
from .imageidentity import (
    exact_image_id,
    exact_image_reference,
    require_exact_image,
    require_input_key,
)
from .invocation import ConsoleMode
from .proc import Runner
from .storage import Storage

INPUT_KEY_LABEL = "org.capsem.package-builder.input-key"


@dataclass(frozen=True)
class PackageBuilderIdentity:
    input_key: str
    image_id: str
    image_reference: str


def _exact_image_id(docker: Docker, image: str, *, platform: str | None = None) -> str:
    return exact_image_id(
        docker,
        image,
        platform=platform,
        subject="package helper dependency",
    )


def _exact_image_reference(
    docker: Docker,
    image: str,
    *,
    platform: str | None = None,
    expected_id: str | None = None,
) -> str:
    expected = expected_id or _exact_image_id(docker, image, platform=platform)
    return exact_image_reference(
        docker,
        image,
        platform=platform,
        expected_id=expected,
        subject="package helper dependency",
    )


def _identity_files(config: GateConfig) -> tuple[Path, ...]:
    settings = config.package.builder
    explicit = tuple(config.path(name) for name in settings.identity_inputs)
    expanded = tuple(
        path for pattern in settings.identity_globs for path in sorted(config.root.glob(pattern))
    )
    files = (*explicit, *expanded)
    missing = [path for path in files if not path.is_file()]
    if missing:
        raise GateError(
            "package helper identity inputs are missing: "
            + ", ".join(str(path) for path in missing)
        )
    return files


def image_tag(
    config: GateConfig,
    target: Arch,
    docker: Docker,
    *,
    parent_id: str | None = None,
) -> str:
    """Input-keyed tag including the mutable host builder's exact identity."""
    settings = config.package.builder
    host_arch = config.host_arch()
    ort = config.toolchain.ort.distributions[target.rust_target]
    digest = hashlib.blake2b(digest_size=16)
    for path in _identity_files(config):
        digest.update(path.relative_to(config.root).as_posix().encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    for value in (
        parent_id
        or _exact_image_id(
            docker,
            config.package.builder_image,
            platform=host_arch.docker_platform,
        ),
        host_arch.name,
        host_arch.docker_platform,
        host_arch.rust_target,
        target.name,
        target.rust_target,
        target.dpkg,
        target.gnu,
        ort.url,
        ort.sha256,
        settings.materialize_build_network,
        settings.source_build_network,
        settings.runtime_network,
        config.apt_snapshot.base,
        config.apt_snapshot.id,
        " ".join(config.toolchain.linux.cross_dev_packages),
        settings.cargo_store,
        settings.pnpm_store,
        settings.ort_lib_location,
    ):
        digest.update(value.encode())
        digest.update(b"\0")
    return settings.tag_template.format(arch=target.name, digest=digest.hexdigest())


def image_repository(config: GateConfig, target: Arch) -> str:
    return config.package.builder.tag_template.split(":", 1)[0].format(arch=target.name)


def _require_input_key(docker: Docker, tag: str) -> None:
    require_input_key(
        docker,
        tag,
        label=INPUT_KEY_LABEL,
        subject="package helper",
    )


def materialize(runner: Runner, config: GateConfig, target: Arch) -> PackageBuilderIdentity:
    """Build a host-native helper for one target, then record its exact ID."""
    settings = config.package.builder
    docker = Docker(runner)
    host_arch = config.host_arch()
    parent_id = _exact_image_id(
        docker,
        config.package.builder_image,
        platform=host_arch.docker_platform,
    )
    parent_reference = _exact_image_reference(
        docker,
        config.package.builder_image,
        platform=host_arch.docker_platform,
        expected_id=parent_id,
    )
    tag = image_tag(config, target, docker, parent_id=parent_id)
    runner.step(f"Materializing locked package dependencies ({target.name})")
    if docker.image_exists(tag, platform=host_arch.docker_platform):
        _require_input_key(docker, tag)
        runner.note(f"package helper input key is already present: {tag}")
    else:
        ort = config.toolchain.ort.distributions[target.rust_target]
        docker.build(
            tag=tag,
            dockerfile=config.path(settings.dockerfile).as_posix(),
            context=str(config.root),
            args=[
                f"BASE={parent_reference}",
                f"RUST_TARGET={target.rust_target}",
                f"HOST_RUST_TARGET={host_arch.rust_target}",
                f"DPKG_ARCH={target.dpkg}",
                f"APT_SNAPSHOT_BASE={config.apt_snapshot.base}",
                f"APT_SNAPSHOT_ID={config.apt_snapshot.id}",
                "CROSS_DEV_PACKAGES=" + " ".join(config.toolchain.linux.cross_dev_packages),
                f"CARGO_STORE={settings.cargo_store}",
                f"PNPM_STORE={settings.pnpm_store}",
                f"ORT_URL={ort.url}",
                f"ORT_SHA256={ort.sha256}",
                f"ORT_LIB_LOCATION={settings.ort_lib_location}",
                f"INPUT_IDENTITY={tag}",
            ],
            platform=host_arch.docker_platform,
            network=settings.materialize_build_network,
            console=ConsoleMode.LOG_ONLY,
        )
        require_exact_image(
            docker,
            parent_reference,
            platform=host_arch.docker_platform,
            expected_id=parent_id,
            subject="package helper dependency during materialization",
        )
        _require_input_key(docker, tag)
    exact_id = _exact_image_id(docker, tag, platform=host_arch.docker_platform)
    exact_reference = _exact_image_reference(
        docker,
        tag,
        platform=host_arch.docker_platform,
        expected_id=exact_id,
    )
    runner.note(
        f"Package helper {target.name}: input key {tag}; exact image {exact_id}; "
        f"build reference {exact_reference}"
    )
    Storage(runner).reclaim(image_repository(config, target), keep=tag)
    return PackageBuilderIdentity(
        input_key=tag,
        image_id=exact_id,
        image_reference=exact_reference,
    )


def require_local_image(runner: Runner, config: GateConfig, target: Arch) -> str:
    """Resolve and verify the input-keyed local FROM tag without warming it.

    The source build is network-denied, so its already-local, input-keyed tag
    is the portable FROM reference on Docker and Colima. The exact child and
    any available repository digest are verified before returning that tag.
    """
    docker = Docker(runner)
    host_arch = config.host_arch()
    tag = image_tag(config, target, docker)
    if not docker.image_exists(tag, platform=host_arch.docker_platform):
        raise GateError(f"package helper {tag} is missing; its materialize step did not complete")
    _require_input_key(docker, tag)
    exact_id = _exact_image_id(docker, tag, platform=host_arch.docker_platform)
    exact_reference = _exact_image_reference(
        docker,
        tag,
        platform=host_arch.docker_platform,
        expected_id=exact_id,
    )
    runner.note(
        f"Using package helper {target.name}: input key {tag}; exact image {exact_id}; "
        f"build reference {exact_reference}"
    )
    return tag

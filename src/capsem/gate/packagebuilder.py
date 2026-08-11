"""Materialize the network-open dependency helper for one Linux package."""

from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass
from pathlib import Path

from .config import Arch, GateConfig
from .docker import Docker
from .errors import GateError
from .invocation import ConsoleMode
from .proc import Runner
from .storage import Storage

INPUT_KEY_LABEL = "org.capsem.package-builder.input-key"


@dataclass(frozen=True)
class PackageBuilderIdentity:
    input_key: str
    image_id: str


def _exact_image_id(docker: Docker, image: str) -> str:
    found = docker.image_id(image)
    if re.fullmatch(r"sha256:[0-9a-f]{64}", found) is None:
        raise GateError(f"package helper dependency {image} has invalid image ID {found!r}")
    return found


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


def image_tag(config: GateConfig, target: Arch, docker: Docker) -> str:
    """Input-keyed tag including the mutable host builder's exact identity."""
    settings = config.package.builder
    host_arch = config.host_arch()
    ort = settings.targets[target.name]
    digest = hashlib.blake2b(digest_size=16)
    for path in _identity_files(config):
        digest.update(path.relative_to(config.root).as_posix().encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    for value in (
        _exact_image_id(docker, config.package.builder_image),
        host_arch.name,
        host_arch.docker_platform,
        target.name,
        target.rust_target,
        target.dpkg,
        target.gnu,
        ort.ort_url,
        ort.ort_sha256,
        settings.materialize_network,
        settings.runtime_network,
        settings.apt_snapshot_base,
        settings.apt_snapshot_id,
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
    found = docker.image_label(tag, INPUT_KEY_LABEL)
    if found != tag:
        raise GateError(
            f"package helper tag {tag} carries input key {found!r}; refusing a poisoned warm tag"
        )


def materialize(runner: Runner, config: GateConfig, target: Arch) -> PackageBuilderIdentity:
    """Build a host-native helper for one target, then record its exact ID."""
    settings = config.package.builder
    docker = Docker(runner)
    host_arch = config.host_arch()
    parent_id = _exact_image_id(docker, config.package.builder_image)
    tag = image_tag(config, target, docker)
    runner.step(f"Materializing locked package dependencies ({target.name})")
    if docker.image_exists(tag, platform=host_arch.docker_platform):
        _require_input_key(docker, tag)
        runner.note(f"package helper input key is already present: {tag}")
    else:
        ort = settings.targets[target.name]
        docker.build(
            tag=tag,
            dockerfile=config.path(settings.dockerfile).as_posix(),
            context=str(config.root),
            args=[
                f"BASE={parent_id}",
                f"RUST_TARGET={target.rust_target}",
                f"DPKG_ARCH={target.dpkg}",
                f"APT_SNAPSHOT_BASE={settings.apt_snapshot_base}",
                f"APT_SNAPSHOT_ID={settings.apt_snapshot_id}",
                f"CARGO_STORE={settings.cargo_store}",
                f"PNPM_STORE={settings.pnpm_store}",
                f"ORT_URL={ort.ort_url}",
                f"ORT_SHA256={ort.ort_sha256}",
                f"ORT_LIB_LOCATION={settings.ort_lib_location}",
                f"INPUT_KEY={tag}",
            ],
            platform=host_arch.docker_platform,
            network=settings.materialize_network,
            console=ConsoleMode.LOG_ONLY,
        )
        _require_input_key(docker, tag)
    exact_id = _exact_image_id(docker, tag)
    runner.note(f"Package helper {target.name}: input key {tag}; exact image {exact_id}")
    Storage(runner).reclaim(image_repository(config, target), keep=tag)
    return PackageBuilderIdentity(input_key=tag, image_id=exact_id)


def require_image_id(runner: Runner, config: GateConfig, target: Arch) -> str:
    """Resolve the already-materialized helper, never warming inside the lane."""
    docker = Docker(runner)
    tag = image_tag(config, target, docker)
    if not docker.image_exists(tag, platform=config.host_arch().docker_platform):
        raise GateError(f"package helper {tag} is missing; its materialize step did not complete")
    _require_input_key(docker, tag)
    exact_id = _exact_image_id(docker, tag)
    runner.note(f"Using package helper {target.name}: input key {tag}; exact image {exact_id}")
    return exact_id

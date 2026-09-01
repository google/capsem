"""Materialize the network-open dependency helper for install qualification."""

from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass
from pathlib import Path

from . import imagecache
from .cachecontrol import CacheControl
from .config import GateConfig
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

INPUT_KEY_LABEL = "org.capsem.install-builder.input-key"


@dataclass(frozen=True)
class InstallBuilderIdentity:
    input_key: str
    image_id: str
    image_reference: str


def _identity_files(config: GateConfig) -> tuple[Path, ...]:
    settings = config.install.builder
    explicit = tuple(config.path(name) for name in settings.identity_inputs)
    expanded = tuple(
        path for pattern in settings.identity_globs for path in sorted(config.root.glob(pattern))
    )
    files = (*explicit, *expanded)
    missing = [path for path in files if not path.is_file()]
    if missing:
        raise GateError(
            "install helper identity inputs are missing: "
            + ", ".join(str(path) for path in missing)
        )
    return files


def _packages(config: GateConfig) -> tuple[str, ...]:
    packages = config.install.builder.apt_packages
    invalid = [name for name in packages if re.fullmatch(r"[a-z0-9][a-z0-9+.-]*", name) is None]
    if not packages or invalid or len(packages) != len(set(packages)):
        raise GateError(
            "install helper apt_packages must be a non-empty unique list of package names"
        )
    return packages


def image_tag(config: GateConfig, docker: Docker, *, parent_id: str | None = None) -> str:
    settings = config.install.builder
    host_arch = config.host_arch()
    digest = hashlib.blake2b(digest_size=16)
    for path in _identity_files(config):
        digest.update(path.relative_to(config.root).as_posix().encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    for value in (
        parent_id
        or exact_image_id(
            docker,
            config.package.builder_image,
            platform=host_arch.docker_platform,
            subject="install helper dependency",
        ),
        host_arch.name,
        host_arch.docker_platform,
        host_arch.rust_target,
        config.apt_snapshot.base,
        config.apt_snapshot.id,
        config.install.venv,
        settings.cargo_store,
        settings.pnpm_store,
        settings.apt_lists_cache_id,
        settings.apt_archives_cache_id,
        settings.materialize_build_network,
        settings.source_build_network,
        *(_packages(config)),
        *(config.install.package_runtime_packages),
    ):
        digest.update(value.encode())
        digest.update(b"\0")
    return settings.tag_template.format(arch=host_arch.name, digest=digest.hexdigest())


def image_repository(config: GateConfig) -> str:
    return config.install.builder.tag_template.split(":", 1)[0].format(arch=config.host_arch().name)


def _require_key(docker: Docker, tag: str) -> None:
    require_input_key(
        docker,
        tag,
        label=INPUT_KEY_LABEL,
        subject="install helper",
    )


def materialize(runner: Runner, config: GateConfig) -> InstallBuilderIdentity:
    """Build or reuse the exact host-native helper and record its identity."""
    settings = config.install.builder
    docker = Docker(runner)
    host_arch = config.host_arch()
    parent_id = exact_image_id(
        docker,
        config.package.builder_image,
        platform=host_arch.docker_platform,
        subject="install helper dependency",
    )
    parent_reference = exact_image_reference(
        docker,
        config.package.builder_image,
        platform=host_arch.docker_platform,
        expected_id=parent_id,
        subject="install helper dependency",
    )
    tag = image_tag(config, docker, parent_id=parent_id)
    runner.step(f"Materializing locked install dependencies ({host_arch.name})")
    if docker.image_exists(tag, platform=host_arch.docker_platform):
        _require_key(docker, tag)
        runner.note(f"install helper input key is already present: {tag}")
    else:
        docker.build(
            tag=tag,
            dockerfile=str(config.path(settings.dockerfile)),
            context=str(config.root),
            args=[
                f"BASE={parent_reference}",
                f"APT_SNAPSHOT_BASE={config.apt_snapshot.base}",
                f"APT_SNAPSHOT_ID={config.apt_snapshot.id}",
                f"APT_PACKAGES={' '.join(_packages(config))}",
                f"RUST_TARGET={host_arch.rust_target}",
                f"INSTALL_VENV={config.install.venv}",
                f"CARGO_STORE={settings.cargo_store}",
                f"PNPM_STORE={settings.pnpm_store}",
                f"APT_LISTS_CACHE_ID={settings.apt_lists_cache_id}",
                f"APT_ARCHIVES_CACHE_ID={settings.apt_archives_cache_id}",
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
            subject="install helper dependency during materialization",
        )
        _require_key(docker, tag)
    exact_id = exact_image_id(
        docker,
        tag,
        platform=host_arch.docker_platform,
        subject="install helper",
    )
    reference = exact_image_reference(
        docker,
        tag,
        platform=host_arch.docker_platform,
        expected_id=exact_id,
        subject="install helper",
    )
    runner.note(
        f"Install helper {host_arch.name}: input key {tag}; exact image {exact_id}; "
        f"build reference {reference}"
    )
    CacheControl(runner).reclaim(
        image_repository(config),
        keep=tag,
        protect=imagecache.protected_tags(
            config,
            image_repository(config),
            field="helper_input_key",
        ),
    )
    return InstallBuilderIdentity(tag, exact_id, reference)


def require_local_image(
    runner: Runner,
    config: GateConfig,
    *,
    expected: InstallBuilderIdentity,
) -> str:
    """Revalidate the exact local helper immediately before sealed FROM."""
    docker = Docker(runner)
    platform = config.host_arch().docker_platform
    tag = image_tag(config, docker)
    if tag != expected.input_key:
        raise GateError(f"install helper {expected.input_key} is no longer selected")
    _require_key(docker, tag)
    found = exact_image_id(docker, tag, platform=platform, subject="install helper")
    if found != expected.image_id:
        raise GateError(
            f"install helper {tag} moved before the sealed source build: "
            f"expected {expected.image_id}, found {found}"
        )
    require_exact_image(
        docker,
        expected.image_reference,
        platform=platform,
        expected_id=expected.image_id,
        subject="install helper build reference",
    )
    return tag


def require_current(runner: Runner, config: GateConfig) -> InstallBuilderIdentity:
    """Resolve the current input key and prove its exact local image still exists."""
    docker = Docker(runner)
    platform = config.host_arch().docker_platform
    tag = image_tag(config, docker)
    if not docker.image_exists(tag, platform=platform):
        raise GateError(f"install helper {tag} is missing; its materialize step did not complete")
    _require_key(docker, tag)
    found = exact_image_id(docker, tag, platform=platform, subject="install helper")
    reference = exact_image_reference(
        docker,
        tag,
        platform=platform,
        expected_id=found,
        subject="install helper",
    )
    return InstallBuilderIdentity(tag, found, reference)

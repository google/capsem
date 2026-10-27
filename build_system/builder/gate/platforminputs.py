"""Source-keyed distribution prerequisites, prepared before offline probes."""

from __future__ import annotations

from .. import crypto
from .config import GateConfig
from .imagebases import build_config
from .platformschema import LinuxDistribution


def prepared_image(config: GateConfig, row: LinuxDistribution) -> str:
    if row.probe_suite is None:
        return row.image()
    settings = config.platforms.linux.probe_inputs
    build = build_config(config)
    arch = config.host_arch()
    truststore = build.architectures[arch.name].rust_builder_base_image
    # Cache deduplication identity, not an authenticity claim: every build
    # consumes digest-pinned bases, signed snapshot packages and this Dockerfile.
    digest = crypto.blake3()
    for value in (
        row.image(),
        row.probe_suite,
        arch.docker_platform,
        settings.materialize_network.value,
        truststore,
        build.asset_tools.debian_snapshot_base,
        build.asset_tools.debian_snapshot_id,
        *config.install.package_preinstall_packages,
    ):
        digest.update(value.encode())
        digest.update(b"\0")
    digest.update(config.path(settings.dockerfile).read_bytes())
    return settings.tag_template.format(
        version=row.version, arch=arch.name, digest=digest.hexdigest()
    )


def commands(config: GateConfig) -> tuple[tuple[str, ...], ...]:
    arch = config.host_arch()
    rows = config.platforms.linux.distributions
    result: list[tuple[str, ...]] = [
        ("docker", "pull", "--platform", arch.docker_platform, row.image()) for row in rows
    ]
    if not any(row.probe_suite for row in rows):
        return tuple(result)
    build = build_config(config)
    truststore = build.architectures[arch.name].rust_builder_base_image
    result.append(("docker", "pull", "--platform", arch.docker_platform, truststore))
    settings = config.platforms.linux.probe_inputs
    for row in rows:
        if row.probe_suite is None:
            continue
        arguments = (
            f"BASE_IMAGE={row.image()}",
            f"TRUSTSTORE_IMAGE={truststore}",
            f"DEBIAN_SNAPSHOT_BASE={build.asset_tools.debian_snapshot_base}",
            f"DEBIAN_SNAPSHOT_ID={build.asset_tools.debian_snapshot_id}",
            f"PROBE_SUITE={row.probe_suite}",
            f"PROBE_PACKAGES={' '.join(config.install.package_preinstall_packages)}",
        )
        result.append(
            (
                "docker",
                "build",
                "--platform",
                arch.docker_platform,
                "--network",
                settings.materialize_network.value,
                "-t",
                prepared_image(config, row),
                "-f",
                str(config.path(settings.dockerfile)),
                *(part for value in arguments for part in ("--build-arg", value)),
                str(config.root),
            )
        )
    return tuple(result)


def proof_arguments(config: GateConfig) -> tuple[str, ...]:
    return tuple(
        part
        for row in config.platforms.linux.distributions
        if row.probe_suite is not None
        for part in ("--probe-image", row.image(), prepared_image(config, row))
    )

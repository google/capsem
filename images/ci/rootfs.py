"""Turn one built official image into its published rootfs evidence.

Exports the image's flattened filesystem once, then:

- inventories it as a CycloneDX OBOM with the same pinned cdxgen, normalizer
  and validator the VM guest rootfs uses, naming the image by repository and
  per-architecture manifest digest;
- packs it as EROFS with the VM rootfs compression settings, every owner and
  group shifted by WORKLOAD_ID_OFFSET so the files land owned by the
  workload's user namespace rather than by host-side root.

Both run in the offline asset-tools image, built here from the same
config-owned inputs the gate uses.

Usage: rootfs.py --image TAG --arch {arm64,amd64} --repository REPO --digest SHA --out DIR
"""

from __future__ import annotations

import argparse
import subprocess
from collections.abc import Sequence
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.image import assettools, docker
from capsem_builder.image.config import load_guest_config
from capsem_builder.image.models import BuildConfig
from capsem_builder.release.obom import ObomSubject

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = "docker"
#: The first host id of the workload's user namespace: image root (uid 0)
#: maps to it, and the image's `capsem` user (uid 1000) to 101000.
WORKLOAD_ID_OFFSET = 100000


def build_config() -> BuildConfig:
    config = gate_config.load(ROOT)
    return load_guest_config(config.path(config.imagebuild.source_config)).build


def builder_architecture(build: BuildConfig, arch: str) -> str:
    """The guest config's name for an OCI architecture (`amd64` is `x86_64`)."""
    matches = [
        name
        for name, settings in build.architectures.items()
        if settings.docker_platform == f"linux/{arch}"
    ]
    if len(matches) != 1:
        raise ValueError(f"expected one guest architecture for linux/{arch}, got {matches}")
    return matches[0]


def asset_tools(build: BuildConfig, name: str) -> str:
    """The input-keyed asset tools image, built if this runner lacks it."""
    tag = assettools.image_tag(build, name, ROOT)
    present = (
        subprocess.run(
            [RUNTIME, "image", "inspect", tag], capture_output=True, check=False
        ).returncode
        == 0
    )
    if not present:
        arguments = assettools.build_arguments(build, name, tag)
        docker.docker_build(
            RUNTIME,
            tag,
            ROOT / build.asset_tools.dockerfile,
            ROOT,
            build.architectures[name].docker_platform,
            network=build.asset_tools.materialize_network,
            build_args=dict(argument.split("=", 1) for argument in arguments),
        )
    return tag


def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--image", required=True, help="local tag of the built image")
    parser.add_argument("--arch", required=True, choices=("arm64", "amd64"))
    parser.add_argument("--repository", required=True)
    parser.add_argument("--digest", required=True, help="the pushed per-arch manifest digest")
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)

    build = build_config()
    name = builder_architecture(build, args.arch)
    platform = build.architectures[name].docker_platform
    tools = asset_tools(build, name)
    network = build.asset_tools.runtime_network
    args.out.mkdir(parents=True, exist_ok=True)
    tar = args.out / "rootfs.tar"
    docker.export_container_fs(RUNTIME, args.image, platform, tar)
    try:
        docker.generate_cyclonedx_obom(
            tar,
            args.out / "obom.cdx.json",
            repo_root=ROOT,
            architecture=args.arch,
            runtime=RUNTIME,
            tool_image=tools,
            tool_platform=platform,
            runtime_network=network,
            subject=ObomSubject(name=args.repository, version=args.digest),
        )
        _, compression, cluster_size, level = docker.experimental_erofs_build_config(
            {}, build.erofs
        )
        docker.create_erofs(
            RUNTIME,
            tar,
            args.out / "rootfs.erofs",
            compression,
            cluster_size,
            level,
            tool_image=tools,
            runtime_network=network,
            id_offset=WORKLOAD_ID_OFFSET,
        )
    finally:
        tar.unlink(missing_ok=True)


if __name__ == "__main__":
    main()

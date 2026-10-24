"""Gate official evidence/index activation on the original native image/root.

Platform bytes already exist in the registry. This is not atomic publication:
failure leaves those immutable candidate bytes, but cannot advance the catalog.
The top-level gate owns its own machine lock, runtime, journal and teardown.
"""

from __future__ import annotations

import argparse
import os
import stat
from pathlib import Path

from capsem_builder.cache.config import load_paths
from capsem_builder.cache.leases import release_path, retain_generation
from capsem_builder.gate.config import load
from capsem_builder.gate.proc import Runner

from images.ci.catalog import descriptions
from tests.fixtures.oci import pinned_image, producer_evidence, published_root
from tests.fixtures.oci.native_candidates import contained, host_platform
from tests.fixtures.oci.native_transfer import capacity

ROOT = Path(__file__).resolve().parents[2]


def preflight(architecture: str) -> None:
    if architecture not in ("arm64", "amd64") or host_platform() != "linux/" + architecture:
        raise ValueError("official qualification requires the actual native Linux architecture")
    for name in ("/dev/kvm", "/dev/vhost-vsock"):
        path = Path(name)
        if (
            not path.exists()
            or not stat.S_ISCHR(path.stat().st_mode)
            or not os.access(path, os.R_OK | os.W_OK)
        ):
            raise ValueError(f"official native qualification requires accessible {name}")


def qualify(config, image, architecture, repository, digest, output, *, runner=None):
    preflight(architecture)
    namespace = config.functional.reference_image.repository.rsplit("/", 1)[0]
    if image not in descriptions() or repository != namespace + "/" + image:
        raise ValueError("official qualification repository/image differs from the catalog")
    if published_root.DIGEST.fullmatch(digest) is None:
        raise ValueError("official qualification requires one immutable platform subject")
    settings = config.functional.reference_image
    paths = load_paths(config.root)
    stage = settings.cache_stage
    key = "official-" + image + "-" + digest.split(":")[1]
    layout = contained(paths.stage(stage), paths.stage(stage) / key)
    template = paths.policy.stages[stage].lease_template
    if template is None:
        raise ValueError("official qualification needs the existing generation lease")
    retain_generation(paths, stage, key)
    try:
        layout.parent.mkdir(parents=True, exist_ok=True)
        capacity(paths, stage, additional_entries=0 if layout.exists() else 1)
        if not layout.exists():
            pinned_image.pull(digest, "linux/" + architecture, layout, repository=repository)
        pinned_image.verify(layout, digest, "linux/" + architecture)
        producer_evidence.stage(
            layout, output, repository, architecture, maximum=settings.evidence_max_bytes
        )
        capacity(paths, stage)
        # No outer machine lease, nested plan or --native-report: this ordinary
        # owner works on BOTH native arches; AMD transfer remains a separate rail.
        (runner or Runner(config.root)).run(
            [
                "uv",
                "run",
                "--project",
                "build_system",
                "--frozen",
                "capsem-gate",
                "image-qualify",
                "--image",
                image,
                "--layout",
                str(layout),
                "--slow",
            ],
            cwd=config.root,
        )
        pinned_image.verify(layout, digest, "linux/" + architecture)
        # Refuse changed original root/evidence after qualification; never repack.
        producer_evidence.stage(
            layout, output, repository, architecture, maximum=settings.evidence_max_bytes
        )
        capacity(paths, stage)
    finally:
        release_path(paths.stage(stage) / template.format(key=key))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--image")
    parser.add_argument("--arch", required=True, choices=("arm64", "amd64"))
    parser.add_argument("--repository")
    parser.add_argument("--digest")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--preflight", action="store_true")
    args = parser.parse_args(argv)
    if args.preflight:
        if any(
            value is not None for value in (args.image, args.repository, args.digest, args.output)
        ):
            parser.error("--preflight accepts only --arch")
        preflight(args.arch)
        return
    if any(value is None for value in (args.image, args.repository, args.digest, args.output)):
        parser.error("qualification requires image, repository, digest and original output")
    qualify(load(ROOT), args.image, args.arch, args.repository, args.digest, args.output)


if __name__ == "__main__":
    main()

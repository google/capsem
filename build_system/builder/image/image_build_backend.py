"""Private image build backend invoked by capsem-admin.

This module is intentionally not exposed as a `capsem-builder` CLI command.
`capsem-admin image build` owns the public runtime image-build rail;
the Python backend only executes the already-materialized guest workspace.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from ..cache import dockercurrent
from ..cache.config import load_paths
from .assetdependencies import AssetDependencyImage
from .config import load_guest_config
from .docker import (
    build_image,
    detect_runtime,
    materialize_asset_dependencies,
    require_asset_dependencies,
)


def declare_current(config, image: AssetDependencyImage, repo_root: Path) -> None:
    """Declare the runtime's dependency image this checkout's current one.

    A dependency repository holds one current tag per runtime image, so
    retention that kept the newest tag per repository removed an image in use.
    """
    root = repo_root.absolute()
    slot = config.manifest.name if config.manifest else "unscoped"
    dockercurrent.record(load_paths(root), tag=image.reference, checkout=root, slot=slot)


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="python -m capsem_builder.image.image_build_backend",
        description="Private Capsem image build backend.",
    )
    parser.add_argument("guest_dir", type=Path)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--template", required=True, choices=("kernel", "rootfs"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--materialize-dependencies", action="store_true")
    parser.add_argument("--require-dependencies", action="store_true")
    args = parser.parse_args()

    config = load_guest_config(args.guest_dir)
    if args.materialize_dependencies and args.require_dependencies:
        parser.error("dependency materialization and verification are mutually exclusive")
    if args.materialize_dependencies:
        resolved = materialize_asset_dependencies(
            config,
            args.arch,
            template=args.template,
            repo_root=Path.cwd(),
        )
        declare_current(config, resolved, Path.cwd())
        print(resolved.image_id)
    elif args.require_dependencies:
        resolved = require_asset_dependencies(
            detect_runtime(),
            config,
            args.arch,
            args.template,
        )
        declare_current(config, resolved, Path.cwd())
        print(resolved.image_id)
    else:
        if args.output is None:
            parser.error("--output is required for an image build")
        build_image(
            config,
            args.arch,
            template=args.template,
            output_dir=args.output,
            repo_root=Path.cwd(),
        )


if __name__ == "__main__":
    main()

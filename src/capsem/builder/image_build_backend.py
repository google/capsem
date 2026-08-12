"""Private image build backend invoked by capsem-admin.

This module is intentionally not exposed as a `capsem-builder` CLI command.
`capsem-admin image build` owns the public profile-derived image-build rail;
the Python backend only executes the already-materialized guest workspace.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from capsem.builder.config import load_guest_config
from capsem.builder.docker import (
    build_image,
    detect_runtime,
    materialize_asset_dependencies,
    require_asset_dependencies,
)


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="python -m capsem.builder.image_build_backend",
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
        print(
            materialize_asset_dependencies(
                config,
                args.arch,
                template=args.template,
                repo_root=Path.cwd(),
            )
        )
    elif args.require_dependencies:
        print(
            require_asset_dependencies(
                detect_runtime(),
                config,
                args.arch,
                args.template,
            )
        )
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

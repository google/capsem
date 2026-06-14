"""Private image build backend invoked by capsem-admin.

This module is intentionally not exposed as a `capsem-builder` CLI command.
`capsem-admin image build` owns the public profile-derived image-build rail;
the Python backend only executes the already-materialized guest workspace.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from capsem.builder.config import load_guest_config
from capsem.builder.docker import build_image


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="python -m capsem.builder.image_build_backend",
        description="Private Capsem image build backend.",
    )
    parser.add_argument("guest_dir", type=Path)
    parser.add_argument("--arch", required=True)
    parser.add_argument("--template", required=True, choices=("kernel", "rootfs"))
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    config = load_guest_config(args.guest_dir)
    build_image(
        config,
        args.arch,
        template=args.template,
        output_dir=args.output,
        repo_root=Path.cwd(),
    )


if __name__ == "__main__":
    main()

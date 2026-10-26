"""python -m capsem_builder.docs_metrics --repo . --output web/docs/public/data/metrics"""

from __future__ import annotations

import argparse
from pathlib import Path

from .export import generate


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--scc", type=Path, help="An already installed source scanner (development only)"
    )
    parser.add_argument(
        "--offline",
        action="store_true",
        help="Use retained GitHub evidence without network requests",
    )
    args = parser.parse_args()
    repo = args.repo.resolve()
    generate(
        repo, args.output or repo / "web/docs/public/data/metrics", args.scc, offline=args.offline
    )


if __name__ == "__main__":
    main()

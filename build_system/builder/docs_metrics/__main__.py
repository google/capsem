"""python -m capsem_builder.docs_metrics --repo . --output web/docs/public/data/metrics"""

from __future__ import annotations

import argparse
import os
from pathlib import Path

from ..gate.config import for_root
from ..gate.sandbox import ENFORCE
from .export import generate
from .tools import source_scanner


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--prepare-tools", action="store_true", help="Materialize verified tools without collecting metrics"
    )
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
    if args.prepare_tools:
        source_scanner(repo)
        return
    # Qualification renders retained GitHub evidence inside its sealed boundary.
    # Ordinary documentation publication still refreshes authenticated evidence.
    offline = args.offline or os.environ.get(for_root(repo).environment.command_sandbox_mode) == ENFORCE
    generate(
        repo, args.output or repo / "web/docs/public/data/metrics", args.scc, offline=offline
    )


if __name__ == "__main__":
    main()

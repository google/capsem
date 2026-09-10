"""Regenerate a gateway SDK contract, or fail on generated-source drift."""

from __future__ import annotations

import argparse
from pathlib import Path

from .generation import python_sources, synchronize, typescript_sources


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--specification", type=Path, required=True)
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--python-package", type=Path)
    target.add_argument("--typescript-source", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    package = args.python_package or args.typescript_source
    sources = python_sources if args.python_package else typescript_sources
    changed = synchronize(package, sources(args.specification), check=args.check)
    for message in changed:
        print(message)
    return 1 if args.check and changed else 0


if __name__ == "__main__":
    raise SystemExit(main())

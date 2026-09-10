"""Regenerate the Python gateway contract, or fail on generated-source drift."""

from __future__ import annotations

import argparse
from pathlib import Path

from .generation import python_sources, synchronize


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--specification", type=Path, required=True)
    parser.add_argument("--python-package", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    changed = synchronize(args.python_package, python_sources(args.specification), check=args.check)
    for message in changed:
        print(message)
    return 1 if args.check and changed else 0


if __name__ == "__main__":
    raise SystemExit(main())

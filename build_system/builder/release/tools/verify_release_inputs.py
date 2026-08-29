"""Verify a previously resolved immutable release-input directory."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from .release_inputs import load_verified_release_inputs


def verify_release_inputs(input_dir: Path) -> dict[str, Any]:
    _, _, verification = load_verified_release_inputs(input_dir)
    return verification


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        report = verify_release_inputs(args.input_dir)
    except (OSError, ValueError) as error:
        print(f"release input verification failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

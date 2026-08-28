#!/usr/bin/env python3
"""List and validate the exact profile axis for a functional release gate.

The selection itself lives in `capsem_builder.gate.profiles`, where the gate can build
a plan from it without a subprocess. This stays as the command-line surface
that CI workflows and the release scripts already call.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from capsem_builder.gate.errors import GateError
from capsem_builder.gate.profiles import declared, materialized


def release_test_profiles(profiles_dir: Path, manifest: Path) -> list[str]:
    """The profile axis, base profile first.

    Kept as a function because two test suites import it directly.
    """
    present = materialized(profiles_dir)
    wanted = declared(manifest)
    if wanted is None:
        wanted = present
    elif set(present) != set(wanted):
        raise ValueError(
            "materialized profile catalog does not match the selected manifest: "
            f"manifest={wanted}, materialized={present}"
        )
    wanted.sort(key=lambda profile_id: (profile_id != "code", profile_id))
    return wanted


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profiles-dir", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    args = parser.parse_args()
    try:
        profiles = release_test_profiles(args.profiles_dir, args.manifest)
    except (ValueError, GateError) as error:
        print(f"release profile test selection failed: {error}", file=sys.stderr)
        return 1
    print("\n".join(profiles))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

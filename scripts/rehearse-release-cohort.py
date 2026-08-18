#!/usr/bin/env python3
"""Build a digest-verified release cohort out of what this machine just built.

`just test` builds every artifact and installs the package it built. What it
never did is the other half of a release: resolve a cohort from a manifest,
verify every byte against the digest that manifest records, stage it, and prove
the package against *that*. Those five steps are the only ones a binary release
runs and a local gate does not, and every defect in them cost a forty-minute
dispatch to find -- seven of them, ending with two glow-up steps that could not
have started at all because three required arguments were never passed.

So the cohort is fabricated here rather than pulled from a channel, and
everything downstream of it is the real thing. The manifest is authored by
`capsem-admin`, the same binary a release uses. The fetch is
`fetch-release-artifacts.py`, the same script the workflow's composite action
calls, reading `file://` URLs it already supports. The staging is
`stage-release-test-inputs.py` and `materialize-config.sh`, the same two the
pairing job runs. Only the URLs are local, and only because a local run has
nowhere else to put bytes it has not published.

What this deliberately does not do is fake the shape. A hand-written manifest
would satisfy the verifier and prove nothing about what `capsem-admin` emits,
which is where the profile-publication duplicates came from in the first place.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))


def main() -> int:
    # Imported here rather than at module scope: the sibling is only importable
    # once `sys.path` has been extended above, and a top-level import after that
    # is an E402 this repository's suppression budget does not have room for.
    from release_cohort import build_cohort

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets-dir", required=True, type=Path)
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--packages-dir", required=True, type=Path)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--inputs-dir", required=True, type=Path)
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--content-root", required=True, type=Path)
    parser.add_argument("--before-inputs", required=True, type=Path)
    parser.add_argument("--channel", required=True)
    print(json.dumps(build_cohort(parser.parse_args()), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

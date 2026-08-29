"""Refuse to release while the working tree has uncommitted changes.

A release qualifies and publishes one immutable commit. It runs from a detached
private copy of that commit, so anything still sitting in the working tree is
not in the release and never was -- the run simply cannot see it.

That is the right design and a quiet trap. Three times in one afternoon a fix
was written, verified by hand, and then released without being committed: the
gate dutifully built the old bytes, the change appeared to have done nothing,
and the next hour went into explaining a result that was correct all along.
Nothing said the tree was dirty, because nothing was looking.

So this looks, first, before any qualification is accepted or any ref is
published. `--force` is there for the case where the difference is deliberate
and understood -- a scratch file, a local experiment -- and it has to be typed,
which is the whole point.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


def dirty_paths(root: Path) -> list[str]:
    """Everything Git reports as changed, staged, or untracked."""
    result = subprocess.run(
        ("git", "status", "--porcelain", "--untracked-files=all"),
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return [line for line in result.stdout.splitlines() if line.strip()]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="the checkout the operator edits")
    parser.add_argument("commit", help="the commit being released")
    args = parser.parse_args(argv)

    changed = dirty_paths(args.root)
    if not changed:
        print(f"working tree clean; releasing {args.commit}")
        return 0

    shown = "\n".join(f"  {line}" for line in changed[:20])
    if len(changed) > 20:
        shown += f"\n  ... and {len(changed) - 20} more"
    raise SystemExit(
        f"{args.root} has {len(changed)} uncommitted change(s), and a release "
        f"publishes {args.commit} rather than the working tree:\n{shown}\n\n"
        "Commit them and release the new commit, or pass --force if the "
        "difference is deliberate."
    )


if __name__ == "__main__":
    sys.exit(main())

"""One audited way to place a file into published output.

A hardlink is the only operation that makes two paths *the same file*, and that
is fine for build output and wrong for checked-in source. `capsem-admin` staged
profile payloads with one and put 48 tracked `config/` files inside published
release artifacts sharing a single inode each -- so a `chmod` on the artifact
rewrote tracked source and no content digest noticed, because the content had
not changed.

That offender was Rust and is guarded by `tests/test_rust_filesystem_chokepoint.py`.
This is the same guarantee on the Python side: `stage` classifies its source
before choosing, and `build_system/tests/gate/test_python_filesystem_chokepoint.py` refuses a raw
`os.link` anywhere else.

**It fails closed.** A source it cannot classify is copied, not linked. The
cheap wrong answer is to link and hope; the expensive one is a release whose
artifacts alias the working tree, and copying costs a few milliseconds.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path


def tracked(path: Path) -> bool | None:
    """Whether git tracks `path`. `None` when the question cannot be answered.

    Three-valued on purpose: "not tracked" and "no idea" must not collapse into
    the same branch, because one of them is the case where linking is safe and
    the other is the case where it might not be.
    """
    try:
        finished = subprocess.run(
            ["git", "ls-files", "--error-unmatch", "--", path.name],
            cwd=path.parent,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return None
    if finished.returncode == 0:
        return True
    # git distinguishes "not tracked" (1) from "not a repository" and every
    # other failure, which is exactly the distinction being preserved.
    if "not match any file" in finished.stderr or finished.returncode == 1:
        return False
    return None


def stage(source: Path, destination: Path) -> None:
    """Place `source` at `destination`, sharing an inode only when that is safe.

    Build output is hardlinked, which is what makes staging cheap. Anything
    tracked by git, or anything whose status cannot be determined, is copied --
    so published output can never alias the working tree.
    """
    destination.parent.mkdir(parents=True, exist_ok=True)
    if tracked(source) is False:
        try:
            os.link(source, destination)
            return
        except OSError:
            # Cross-device, or a filesystem without hardlinks. Copying is the
            # documented fallback and preserves the guarantee either way.
            pass
    shutil.copy2(source, destination)

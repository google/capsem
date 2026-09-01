"""Start the gate on an interpreter that cannot read yesterday's bytecode.

CPython validates a `.pyc` against the source's *mtime and size*. Two edits of
the same length within one timestamp tick therefore leave bytecode that still
looks valid, and the interpreter runs the old one. During a review of this
package that produced 74 identical false failures from a reference the source
no longer contained; an isolated cache made them vanish without a source
change.

That is not merely bad local feedback. `just test` and both release
commands begin with `uv run --project build_system --frozen capsem-gate`. The local diagnostic records its
commit and source digest, while each release freezes and dispatches that exact
source to a hosted qualifying lane. A stale module could otherwise construct a
plan that does not correspond to the source being diagnosed or published.

So the entry point is this file rather than `capsem_builder.gate.cli`: it
re-execs with a per-invocation `pycache_prefix` before anything from
`capsem_builder.gate` is imported. `PYTHONPYCACHEPREFIX` goes into the
environment as well as onto the command line, so every child -- pytest, the
builders, the scripts -- inherits the same isolation.

Nothing here may import `capsem_builder.gate`. Importing it is precisely what
this file exists to do only after the cache is safe, and
`capsem_builder.gate.__init__` carries real code. Only the standard library,
and only a few lines of it, so that this module's own bytecode is something
that never changes.
"""

from __future__ import annotations

import os
import shutil
import sys
import time
from contextlib import suppress
from pathlib import Path
from typing import NoReturn

#: Set once the interpreter is running under an isolated cache. Its presence is
#: how the re-exec knows not to happen twice.
MARKER = "CAPSEM_GATE_PYCACHE"

#: The variable CPython itself reads. Exported rather than only passed as `-X`,
#: because the point is that children inherit it.
PYCACHE = "PYTHONPYCACHEPREFIX"

#: Where the per-invocation caches live, under the tree the gate already
#: reclaims. Spelled here rather than read from `config/gate.toml`, because
#: loading that config means importing the package this runs before.
ROOT = "cache/target/gate-pycache"

#: A prefix older than this belongs to an invocation that is long over. Pruned
#: opportunistically on the way in, which keeps the directory bounded without
#: needing a lock or a teardown that a killed process would skip.
STALE_SECONDS = 6 * 3600


def checkout() -> Path:
    """The checkout this launcher was installed from.

    `capsem_builder.gate.project_root` answers the same question and validates
    the answer, which is the better version -- and unreachable from here,
    because importing it is the thing being deferred.
    """
    return Path(__file__).resolve().parents[2]


def _prune(root: Path) -> None:
    cutoff = time.time() - STALE_SECONDS
    with suppress(OSError):
        for entry in root.iterdir():
            # Tolerated, unlike every other removal in this package: another
            # interpreter may be writing into it right now, and the only cost
            # of leaving it is that the next invocation tries again.
            if entry.is_dir() and entry.stat().st_mtime < cutoff:
                shutil.rmtree(entry, ignore_errors=True)


def isolated_environment(root: Path | None = None) -> dict[str, str]:
    """A fresh cache prefix, as the environment that selects it.

    Returned rather than applied, so a test can prove the mechanism against a
    module it controls instead of against the gate's own seventy.
    """
    prefix = (root or checkout()) / ROOT
    prefix.mkdir(parents=True, exist_ok=True)
    _prune(prefix)
    # The pid alone is not enough: pids are reused, and a reused one would
    # inherit the cache of the invocation that held it.
    invocation = prefix / f"{os.getpid():d}-{time.time_ns():x}"
    invocation.mkdir()
    return {MARKER: str(invocation), PYCACHE: str(invocation)}


def main() -> int:
    """Re-exec under an isolated cache, then be the gate."""
    if os.environ.get(MARKER):
        from .gate.cli import main as gate

        return gate()

    return _reexec()


def _reexec() -> NoReturn:
    """Become the same command on an interpreter with a private cache.

    `execv`, not a subprocess: a wrapper process would sit between the terminal
    and the gate for the whole run, taking the signals the gate has to handle
    itself.
    """
    os.environ.update(isolated_environment())
    os.execv(sys.executable, [sys.executable, "-m", "capsem_builder.gate", *sys.argv[1:]])
    raise AssertionError("execv returned")  # pragma: no cover - execv does not

"""The child half of the run-log contention tests.

The spawn start method pickles a process target by qualified name and re-imports
its module in the child, and the child has nothing but a copy of the parent's
`sys.path` to find that module with. A pytest test module is not reachable that
way. `--import-mode=importlib` names test modules `tests.<basename>` without
ever putting the repository root on `sys.path`, so the name resolves only when
the parent was started as `python -m pytest` -- which contributes the working
directory -- and not when it was started through the `pytest` console script.
The child died on `ModuleNotFoundError: No module named 'tests'`, the parent
waited out its queue timeout, and the same assertion about the same code passed
or failed depending on how somebody typed the command.

`tests/` is on `sys.path` unconditionally: the root conftest puts it there
before collection, under every invocation and in every xdist worker. A target
that lives here is therefore importable in the child whichever way pytest was
started. Spawn targets for these tests belong in this module, not beside the
test that uses them.
"""

from __future__ import annotations

from multiprocessing.queues import Queue
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate.runlog import RunLog


def open_and_hold(root: str, name: str, ready: Queue, go: Queue) -> None:
    """Open a run log, announce it, and hold it open until told to finish."""
    settings = gate_config.load(Path(root))
    with RunLog.open(settings, name) as log:
        ready.put(log.directory.name)
        go.get(timeout=60)

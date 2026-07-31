"""Every pidfile the gate stops must be one some binary actually writes.

Stopping the process named by `<run_dir>/service.pid` silently succeeds when
that file does not exist, so a cleanup step aimed at a pidfile nobody writes
reaps nothing and reports success. The asset gate did exactly that: it reaped the
gateway, whose pidfile is real, and left a `capsem-service` behind on every
run. Sixteen accumulated in a day, each holding a `capsem-tray`, all reparented
to launchd.

The failure is silent by construction -- a no-op cleanup looks identical to a
successful one -- so it is invisible until somebody counts processes.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
CRATES = PROJECT_ROOT / "crates"


def _pidfiles_the_gate_stops() -> set[str]:
    """The names the gate reaps, read from where they are declared.

    They were spelled inside the asset recipe when this was shell, so the guard
    parsed them out of the recipe text. They are `[pidfiles] names` in
    `config/gate.toml` now, which is also what `capsem.gate.pidfiles` iterates
    -- so this checks the same list the gate uses rather than a copy of it.
    """
    from capsem.gate import config as gate_config

    return set(gate_config.load(PROJECT_ROOT).pidfiles.names)


def _pidfiles_written_by_a_binary() -> set[str]:
    written: set[str] = set()
    for path in CRATES.rglob("*.rs"):
        text = path.read_text(encoding="utf-8", errors="ignore")
        # A pidfile is written when its path is built and process id stored.
        if "std::process::id()" not in text and "process::id()" not in text:
            continue
        written.update(re.findall(r'["\'](\w+\.pid)["\']', text))
        if "service_pidfile_path" in text:
            written.add("service.pid")
    return written


def test_every_stopped_pidfile_is_written_by_something() -> None:
    stopped = _pidfiles_the_gate_stops()
    assert stopped, "no gate pidfiles declared; this guard would pass vacuously"

    written = _pidfiles_written_by_a_binary()
    orphans = sorted(stopped - written)

    assert not orphans, (
        "the gate stops these pidfiles but no binary writes them, so cleanup "
        "silently reaps nothing and the process survives every run: "
        + ", ".join(orphans)
    )

"""Fail a gate run that ends with capsem processes it started still alive.

`build_system/tests/scripts/test_pidfile_cleanup_is_wired.py` proves the reaping is *wired*: every
pidfile the gate stops is one some binary actually writes. It cannot prove the
reaping *worked*. Those are different claims, and the gap between them is where
this bug lived: a losing service starter deleted the winner's pidfile on its way
out, so `stop_gate_pidfile` found no file, no-opped, and reported success while
the real service ran on under launchd holding a gateway and a tray. Six of them
accumulated across one session of release-lane runs, alive for hours.

A no-op cleanup is indistinguishable from a successful one by construction. The
only way to tell them apart is to count processes afterwards, which is what this
does.

Two modes, both driven from the `test` recipe:

  baseline  snapshot the capsem processes that already existed, so a developer's
            own `just run-service` daemon or an editor's `capsem-mcp` is never
            blamed on the gate.
  check     anything running now that is not in that snapshot was started by
            this run and outlived it. Report it, reap it, fail.

Scope is this repository's own build tree. An installed `~/.capsem/bin`
service belongs to the user, not to the gate, and is never touched.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import signal
import sys
import time
from pathlib import Path

import psutil

PROJECT_ROOT = Path(os.environ.get("CAPSEM_REPOSITORY_ROOT", Path.cwd())).resolve()

# A gate that just finished may still have companions winding down: capsem-guard
# polls parent liveness every 100ms and the service gives its VM children a
# 500ms SIGTERM grace. 15s covers the pessimistic chain without costing anything
# when things settle fast -- the loop exits as soon as the suspects drain.
SETTLE_BUDGET_S = 15.0
REAP_GRACE_S = 5.0


def _process_facts(proc: psutil.Process) -> dict | None:
    """Identity and diagnostics for one process, or None if it isn't ours.

    Reads are per-process and individually guarded. Prefetching via
    `process_iter(attrs=...)` reads every host process's cmdline, and on macOS a
    single sysctl(KERN_PROCARGS2) denial for an unrelated system process
    surfaces as a SystemError that escapes the iterator -- the same failure
    mode tests/conftest.py documents.
    """
    try:
        name = proc.name() or ""
    except (psutil.Error, OSError, SystemError):
        return None
    if not name.startswith("capsem-"):
        return None

    try:
        cmdline = " ".join(proc.cmdline() or [])
    except (psutil.Error, OSError, SystemError):
        cmdline = ""
    try:
        executable = proc.exe() or ""
    except (psutil.Error, OSError, SystemError):
        executable = ""
    try:
        created = proc.create_time()
    except (psutil.Error, OSError, SystemError):
        return None
    try:
        ppid = proc.ppid()
    except (psutil.Error, OSError, SystemError):
        ppid = 0

    return {
        "pid": proc.pid,
        "name": name,
        "cmdline": cmdline,
        "exe": executable,
        "created": created,
        "ppid": ppid,
    }


def _owned_roots(root: Path) -> list[Path]:
    """`root`, plus wherever its own `cache/target/` profile links actually point.

    Compiler output is shared between runs at one absolute path, so a prefix
    reaches it through `cache/target/cargo/debug` and `cache/target/release` symlinks. Resolving
    an exe therefore lands outside the tree that launched it, and a check that
    only compared against `root` stopped recognizing this run's own service --
    which is the failure direction that matters: an unrecognized process is not
    reaped, so a leaked VM or gateway survives the run that started it.

    Derived from the tree rather than from config, because a link is the fact.
    A profile directory that is a real directory contributes nothing, since
    `root` already covers it. Safe against another run's leftovers for the same
    reason the sharing is safe at all: one gate runs per machine under `flock`.
    """
    roots = [root.resolve()]
    for link in sorted((root / "cache" / "target" / "cargo").glob("*")):
        try:
            if link.is_symlink():
                roots.append(link.resolve())
        except OSError:
            continue
    return roots


def _from_this_tree(facts: dict, root: Path) -> bool:
    """True if the binary lives under `root` or a build root `root` points at.

    The gate builds and runs out of the checkout. A service installed under
    `~/.capsem/bin`, or one launched from a different worktree, is somebody
    else's process and must never be reaped by this run's cleanup.
    """
    candidate = facts["exe"] or facts["cmdline"].split(" ", 1)[0]
    if not candidate:
        return False
    try:
        resolved = Path(candidate).resolve()
        return any(resolved.is_relative_to(owned) for owned in _owned_roots(root))
    except (OSError, ValueError):
        return False


def repo_capsem_processes(root: Path = PROJECT_ROOT) -> dict[int, dict]:
    """Every live capsem-* process running from this checkout, keyed by pid."""
    found: dict[int, dict] = {}
    for proc in psutil.process_iter():
        facts = _process_facts(proc)
        if facts is None or not _from_this_tree(facts, root):
            continue
        found[facts["pid"]] = facts
    return found


def started_during_run(current: dict[int, dict], baseline: dict[int, float]) -> dict[int, dict]:
    """Processes in `current` that the baseline did not already account for.

    Start time is part of the identity, not just the pid. A pid recycled onto a
    fresh capsem process during a multi-hour gate would otherwise be waved
    through as pre-existing -- which is precisely a leak we are here to catch.
    """
    leaked = {}
    for pid, facts in current.items():
        known = baseline.get(pid)
        if known is not None and abs(known - facts["created"]) < 0.001:
            continue
        leaked[pid] = facts
    return leaked


def _settle(root: Path, baseline: dict[int, float], budget_s: float) -> dict[int, dict]:
    """Poll until the suspects drain or the budget runs out."""
    deadline = time.monotonic() + budget_s
    delay = 0.05
    while True:
        alive = started_during_run(repo_capsem_processes(root), baseline)
        if not alive or time.monotonic() >= deadline:
            return alive
        time.sleep(min(delay, max(0.0, deadline - time.monotonic())))
        delay = min(delay * 2, 0.5)


def _describe(facts: dict) -> str:
    elapsed = max(0.0, time.time() - facts["created"])
    return (
        f"  pid={facts['pid']} ppid={facts['ppid']} alive={elapsed / 60:.1f}min "
        f"{facts['name']}\n    {facts['cmdline'] or facts['exe']}"
    )


def _reap(leaked: dict[int, dict], root: Path, baseline: dict[int, float]) -> dict[int, dict]:
    """SIGTERM, then SIGKILL survivors. Returns whatever is still alive.

    Signalling exact pids, never a pattern: `pkill -f capsem-` matches
    `--crate-name capsem-core` in a running rustc and would take down a
    concurrent build.
    """
    for pid in leaked:
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.kill(pid, signal.SIGTERM)
    survivors = _settle(root, baseline, REAP_GRACE_S)
    for pid in survivors:
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.kill(pid, signal.SIGKILL)
    return _settle(root, baseline, REAP_GRACE_S)


def write_baseline(path: Path, root: Path) -> dict[int, dict]:
    existing = repo_capsem_processes(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps({str(pid): facts["created"] for pid, facts in existing.items()}),
        encoding="utf-8",
    )
    return existing


def read_baseline(path: Path) -> dict[int, float]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    return {int(pid): float(created) for pid, created in raw.items()}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("baseline", "check"))
    parser.add_argument("--baseline-file", type=Path, required=True)
    # Scoping the scan is what keeps this safe to test. A test that swept the
    # whole checkout could reap a sibling xdist worker's live service.
    parser.add_argument("--root", type=Path, default=PROJECT_ROOT)
    args = parser.parse_args(argv)

    if args.mode == "baseline":
        existing = write_baseline(args.baseline_file, args.root)
        print(
            f"process baseline: {len(existing)} pre-existing capsem process(es) "
            f"recorded in {args.baseline_file}"
        )
        if existing:
            # Say this out loud. Baselining is what keeps the check from blaming
            # this run for someone else's process, but it also means a leak from
            # a previously SIGKILLed run -- where no trap ever ran -- is recorded
            # as pre-existing and never flagged again. Silence would make it
            # permanently invisible, which is how six of them reached five hours
            # old. These also hold ports and sockets the coming run needs.
            print(
                "WARNING: capsem processes from this checkout were already "
                "running before the gate started. They are not this run's and "
                "will not fail it, but a previously interrupted run is the "
                "usual reason, and they can poison this one:",
                file=sys.stderr,
            )
            for facts in sorted(existing.values(), key=lambda f: f["pid"]):
                print(_describe(facts), file=sys.stderr)
        return 0

    if not args.baseline_file.exists():
        print(
            f"ERROR: no process baseline at {args.baseline_file}. The orphan check "
            "cannot tell this run's processes from ones that predate it; run "
            "`check-orphan-processes.py baseline` at the start of the gate.",
            file=sys.stderr,
        )
        return 2

    baseline = read_baseline(args.baseline_file)
    leaked = _settle(args.root, baseline, SETTLE_BUDGET_S)
    if not leaked:
        print("process check: no capsem processes outlived the gate run")
        return 0

    print("\n@@@ ORPHANED CAPSEM PROCESSES SURVIVED THE GATE @@@", file=sys.stderr)
    for facts in sorted(leaked.values(), key=lambda f: f["pid"]):
        print(_describe(facts), file=sys.stderr)

    survivors = _reap(leaked, args.root, baseline)
    if survivors:
        print(
            f"reaped {len(leaked) - len(survivors)}; {len(survivors)} would not die: "
            + ", ".join(str(pid) for pid in sorted(survivors)),
            file=sys.stderr,
        )
    else:
        print(f"reaped {len(leaked)} orphaned process(es)", file=sys.stderr)

    print(
        "A completed or aborted gate run must leave zero capsem processes. Each "
        "one here held its run directory, its gateway, and on macOS its tray, and "
        "would have poisoned the next run's ports and sockets.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())

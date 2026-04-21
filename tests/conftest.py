"""Root conftest: sys.path wiring + artifact capture + leak detection.

Leak detection strategy (xdist-safe):

- Baseline capsem-* PIDs at conftest import time (every process inherits its
  own baseline; pre-existing orphans are never flagged).
- Per-test check_leaks records "first-seen" attribution for any new
  capsem-* PID the worker observes. Writes one JSONL entry per PID to
  tests/leak-attribution.jsonl so attribution survives across worker
  processes.
- pytest_sessionfinish:
    Worker processes (PYTEST_XDIST_WORKER set): do NOT fail, do NOT gate.
    Workers see each other's still-running fixture processes on the host
    and can't reliably tell whose child is whose, so worker-level gating
    false-positives constantly.
    Controller / single-process (PYTEST_XDIST_WORKER unset): this runs
    AFTER every worker has finished, so the host is the source of truth.
    Settle with exponential backoff, filter by baseline, look up
    attribution from the JSONL, fail the session if anything remains.
"""

import json
import os
import sys
import time
from pathlib import Path
import psutil
import pytest

sys.path.insert(0, str(Path(__file__).parent))

# Populated by the hookwrapper below; read by fixtures (ServiceInstance.stop)
# that archive their tmp_dir when this worker session saw any failure.
FAILED_NODEIDS: list[str] = []

# test-artifacts/ at the repo root is the preserve-on-failure destination.
# Gitignored. Fixtures copy their tmp_dir here so service.log /
# sessions/<vm>/process.log / sessions/<vm>/serial.log / session.db all
# survive the normal shutil.rmtree teardown.
ARTIFACTS_ROOT = Path(__file__).parent.parent / "test-artifacts"
LEAK_REPORT_LOG = Path(__file__).parent.parent / "tests" / "leak-report.log"
# Shared cross-process attribution log. Workers append; controller reads.
LEAK_ATTRIBUTION_LOG = Path(__file__).parent.parent / "tests" / "leak-attribution.jsonl"

# PID -> (nodeid, {name, cmdline}). Records the first test this process saw
# each new capsem-* PID alive in. Used to attribute real leaks back to a
# specific test. Per-process state; workers also flush to the shared jsonl
# so the xdist controller can recover attribution at session end.
_FIRST_SEEN: dict[int, tuple[str, dict]] = {}


def _snapshot_baseline_pids() -> set[int]:
    """capsem-* PIDs alive at conftest import time.

    Captured at import so every pytest process (controller, workers, single)
    has a consistent baseline. A fixture-based baseline would miss pre-
    existing orphans in the xdist controller, which never executes session
    fixtures but DOES run pytest_sessionfinish -- it would then flag every
    pre-existing orphan as a leak.
    """
    pids: set[int] = set()
    for proc in psutil.process_iter(['pid', 'name']):
        try:
            name = proc.info['name'] or ''
            if name.startswith('capsem-'):
                pids.add(proc.info['pid'])
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
    return pids


_BASELINE_PIDS: set[int] = _snapshot_baseline_pids()


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    outcome = yield
    rep = outcome.get_result()
    if rep.when in ("setup", "call") and rep.failed:
        FAILED_NODEIDS.append(rep.nodeid)


def _ancestry(pid: int) -> set[int]:
    """Set of ancestor PIDs for `pid`, walked up via psutil until init.

    Excludes `pid` itself. Returns an empty set if `pid` does not exist or
    its parent chain cannot be walked. Robust against processes that die
    mid-walk.
    """
    ancestors: set[int] = set()
    try:
        proc = psutil.Process(pid)
    except (psutil.NoSuchProcess, psutil.AccessDenied):
        return ancestors
    while True:
        try:
            parent = proc.parent()
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            break
        if parent is None:
            break
        ancestors.add(parent.pid)
        proc = parent
    return ancestors


def _is_pytest_descendant(pid: int) -> bool:
    """True if `pid`'s ancestor chain includes this pytest process.

    Used to scope leak detection to processes actually spawned by pytest
    (directly or transitively). Sibling processes that happen to share a
    `capsem-*` name -- e.g. Claude Code's own `capsem-mcp` stdio
    subprocess, or a `capsem-service` started in another shell -- have no
    pytest ancestor and must not be flagged as leaks.
    """
    return os.getpid() in _ancestry(pid)


def get_capsem_processes() -> dict[int, dict]:
    """Return {pid: {name, cmdline}} for every process whose name starts with 'capsem-'.

    Matches on the kernel-reported name only (psutil name()). Scanning cmdline
    args false-positives on every tool invoked from /Users/*/capsem-next/...
    and on any cargo/rustc command that carries `-p capsem-*`.

    cmdline is fetched lazily per capsem-* proc rather than via
    `process_iter(['pid', 'name', 'cmdline'])`. Attr-prefetch reads every
    host proc's cmdline through psutil's as_dict, and on macOS a single
    sysctl(KERN_PROCARGS2) denial for an unrelated system proc surfaces as
    an uncaught SystemError that drops out of process_iter before our
    per-iteration try/except can run. Fetch per-proc, catch per-proc.
    """
    procs: dict[int, dict] = {}
    for proc in psutil.process_iter(['pid', 'name']):
        try:
            name = proc.info['name'] or ''
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
        if not name.startswith('capsem-'):
            continue
        try:
            cmdline = ' '.join(proc.cmdline() or [])
        except (psutil.Error, OSError, SystemError):
            # PermissionError (subclass of OSError) covers KERN_PROCARGS2 denials;
            # SystemError is the psutil C-extension wrapper around the same thing.
            # Either way we know this is a capsem-* proc, so record it with a
            # blank cmdline rather than drop it.
            cmdline = ''
        procs[proc.info['pid']] = {'name': name, 'cmdline': cmdline}
    return procs


def _settle_suspects(
    suspects: dict[int, dict],
    total_budget_s: float,
    initial_delay_s: float,
    max_delay_s: float,
) -> dict[int, dict]:
    """Poll with exponential backoff, dropping suspects as they die.

    Returns whatever's still alive when either the suspect set drains
    (no real leaks) or total_budget_s elapses (real leaks remain). Mirrors
    capsem_core::poll::poll_until -- cheap when things settle quickly,
    bounded when they don't.
    """
    deadline = time.monotonic() + total_budget_s
    delay = initial_delay_s
    alive = dict(suspects)
    while alive:
        time.sleep(min(delay, max(0.0, deadline - time.monotonic())))
        live_now = get_capsem_processes()
        alive = {pid: info for pid, info in alive.items() if pid in live_now}
        if not alive or time.monotonic() >= deadline:
            break
        delay = min(delay * 2, max_delay_s)
    return alive


@pytest.fixture(scope="session", autouse=True)
def _reset_leak_logs():
    """Truncate leak logs at session start so they only reflect this run.

    Only the top-level process should truncate -- if workers truncated,
    they'd wipe each other's attribution data under -n.
    """
    if os.environ.get("PYTEST_XDIST_WORKER") is not None:
        return  # worker: let the controller's truncation stand
    if LEAK_REPORT_LOG.exists():
        LEAK_REPORT_LOG.unlink()
    if LEAK_ATTRIBUTION_LOG.exists():
        LEAK_ATTRIBUTION_LOG.unlink()


@pytest.fixture(autouse=True)
def check_leaks(request):
    """Record first-sighting of each new capsem-* PID for later attribution.

    Does NOT fail per-test. Session-scoped fixtures legitimately keep
    processes alive across many tests; flagging them per-test would drown
    real leaks in noise. The controller's pytest_sessionfinish does the
    actual gate once all workers have finished.
    """
    nodeid = request.node.nodeid
    yield
    worker = os.environ.get("PYTEST_XDIST_WORKER", "master")
    new_records = []
    for pid, info in get_capsem_processes().items():
        if pid in _BASELINE_PIDS:
            continue
        if pid in _FIRST_SEEN:
            continue
        # Scope to pytest's own process tree. Sibling tools on the host
        # (Claude Code's capsem-mcp stdio subprocess, a dev capsem-service
        # running in another shell) also match the capsem-* name filter
        # but aren't ours to flag.
        if not _is_pytest_descendant(pid):
            continue
        _FIRST_SEEN[pid] = (nodeid, info)
        new_records.append({
            "pid": pid,
            "nodeid": nodeid,
            "worker": worker,
            "name": info["name"],
            "cmdline": info["cmdline"],
        })
    if new_records:
        # Append JSONL so workers coexist without locking. Individual line
        # writes are atomic up to PIPE_BUF (4 KiB on macOS/Linux); keep each
        # record under that limit by truncating cmdline below.
        LEAK_ATTRIBUTION_LOG.parent.mkdir(parents=True, exist_ok=True)
        with open(LEAK_ATTRIBUTION_LOG, "a") as f:
            for rec in new_records:
                if len(rec["cmdline"]) > 3000:
                    rec["cmdline"] = rec["cmdline"][:3000] + "...<truncated>"
                f.write(json.dumps(rec) + "\n")


def _load_attribution() -> dict[int, dict]:
    """Merge every worker's first-seen records. Last writer wins per PID."""
    attribution: dict[int, dict] = {}
    if not LEAK_ATTRIBUTION_LOG.exists():
        return attribution
    try:
        with open(LEAK_ATTRIBUTION_LOG) as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                    attribution[int(rec["pid"])] = rec
                except (json.JSONDecodeError, KeyError, ValueError):
                    continue
    except OSError:
        pass
    return attribution


def pytest_sessionfinish(session, exitstatus):
    """Leak gate. Runs in every pytest process; only the controller acts.

    Workers must not gate: each worker sees every other worker's still-
    running fixture processes on the shared host. The xdist controller's
    sessionfinish runs AFTER all workers report done, so the host is the
    source of truth there. Single-process runs hit this path too (no
    PYTEST_XDIST_WORKER env, no PYTEST_XDIST_WORKER_COUNT).
    """
    if os.environ.get("PYTEST_XDIST_WORKER") is not None:
        return  # worker: attribution was flushed in check_leaks

    suspects = {pid: info for pid, info in get_capsem_processes().items() if pid not in _BASELINE_PIDS}
    if not suspects:
        return

    # capsem-guard polls parent liveness every 100ms and exits on death,
    # but companions (gateway, tray, mcp-aggregator, mcp-builtin) may run
    # brief cleanup after guard triggers. 15s covers pessimistic cases
    # (SIGTERM -> 15s ServiceInstance.stop grace -> SIGKILL -> guard
    # detects -> companion exits) without noticeable overhead when things
    # settle fast (loop exits early).
    leaks = _settle_suspects(suspects, total_budget_s=15.0, initial_delay_s=0.05, max_delay_s=0.5)
    if not leaks:
        return

    # Attribution sources: _FIRST_SEEN for same-process runs, jsonl for the
    # xdist controller.
    attribution = _load_attribution()
    for pid, (nodeid, info) in _FIRST_SEEN.items():
        attribution.setdefault(pid, {"nodeid": nodeid, "worker": "master", **info})

    lines = []
    for pid, info in sorted(leaks.items()):
        attrib = attribution.get(pid)
        # Prove the suspect is ours before flagging. Attribution = a per-test
        # check_leaks fixture recorded this PID in its worker; that's the
        # strong signal, and it survives worker exit (the jsonl outlives the
        # worker, whose PID no longer walkable once it's dead). Absent
        # attribution, fall back to an ancestry walk from the controller.
        # A PID with neither attribution nor a pytest ancestor is a sibling
        # tool on the host (Claude Code's capsem-mcp, a manual dev run) --
        # skip it rather than false-positive.
        if attrib is None and not _is_pytest_descendant(pid):
            continue
        origin = attrib["nodeid"] if attrib else "<unknown>"
        worker = attrib["worker"] if attrib else "?"
        lines.append(f"[{worker}] {origin} {pid} {info['name']} {info['cmdline']}\n")

    if not lines:
        return  # every suspect was a sibling process, not ours

    with open(LEAK_REPORT_LOG, "w") as f:
        f.writelines(lines)

    print("\n@@@ CAPSEM PROCESS LEAKS @@@", file=sys.stderr)
    for line in lines:
        print(line.rstrip(), file=sys.stderr)
    print(f"({len(lines)} leaked process(es); see {LEAK_REPORT_LOG})", file=sys.stderr)

    session.exitstatus = 1

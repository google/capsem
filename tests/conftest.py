"""Root conftest: sys.path wiring + artifact capture + leak detection.

Leak detection:
- Baseline snapshot of capsem-* processes at session start.
- Track first-seen nodeid for each new capsem-* PID across the run.
- At session end, report only PIDs that are STILL ALIVE and were not in the
  baseline -- these are the real leaks. Session-scoped fixtures that correctly
  clean up on teardown won't be reported, even though they spawned mid-run.
"""

import os
import sys
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

# PID -> (nodeid, {name, cmdline}). Records the first test where this PID
# was observed alive. Used to attribute real leaks back to a specific test.
_FIRST_SEEN: dict[int, tuple[str, dict]] = {}

# Baseline captured at conftest import time: every capsem-* process alive
# before pytest ran its first test. Populated below (one call per worker
# process, since xdist workers import this module independently). A fixture-
# based baseline would miss pre-existing orphans in the xdist controller,
# which never executes session-scoped fixtures but DOES run
# pytest_sessionfinish -- the controller would then report every pre-
# existing orphan as a leak.
def _snapshot_baseline_pids() -> set[int]:
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


def get_capsem_processes() -> dict[int, dict]:
    """Return {pid: {name, cmdline}} for every process whose name starts with 'capsem-'.

    Matches on the kernel-reported name only (psutil name()). Checking cmdline
    args false-positives on every tool invoked from /Users/*/capsem-next/...
    and on any cargo/rustc command that carries `-p capsem-*`.
    """
    procs: dict[int, dict] = {}
    for proc in psutil.process_iter(['pid', 'name', 'cmdline']):
        try:
            name = proc.info['name'] or ''
            if name.startswith('capsem-'):
                procs[proc.info['pid']] = {
                    'name': name,
                    'cmdline': ' '.join(proc.info['cmdline'] or []),
                }
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
    return procs


@pytest.fixture(scope="session", autouse=True)
def _clear_leak_log():
    """Truncate the leak log at session start so it only reflects this run."""
    if LEAK_REPORT_LOG.exists():
        LEAK_REPORT_LOG.unlink()


@pytest.fixture(autouse=True)
def check_leaks(request):
    """Record first-sighting of each new capsem-* PID for later attribution.

    Does NOT fail per-test. Session-scoped fixtures legitimately keep
    processes alive across many tests; flagging them on every test would
    drown real leaks in noise. The real leak check fires in
    pytest_sessionfinish against processes still alive at the end.
    """
    nodeid = request.node.nodeid
    yield
    for pid, info in get_capsem_processes().items():
        if pid in _BASELINE_PIDS:
            continue
        _FIRST_SEEN.setdefault(pid, (nodeid, info))


def pytest_sessionfinish(session, exitstatus):
    """Report leaks (capsem-* still alive not in baseline) and fail session.

    With pytest-xdist, session fixtures + per-test check_leaks run in WORKER
    processes only. The xdist controller also runs pytest_sessionfinish but
    never populated _FIRST_SEEN or saw the per-test lifecycles, so it can
    only attribute every leak as <unknown>. Let the workers report their own
    leaks (with real attribution) and have the controller stay silent.
    """
    if os.environ.get("PYTEST_XDIST_WORKER") is None and os.environ.get(
        "PYTEST_XDIST_WORKER_COUNT"
    ):
        # Running in the xdist controller -- workers already reported.
        return

    current = get_capsem_processes()
    leaks = {pid: info for pid, info in current.items() if pid not in _BASELINE_PIDS}
    if not leaks:
        return

    worker = os.environ.get("PYTEST_XDIST_WORKER", "master")
    lines = []
    for pid, info in sorted(leaks.items()):
        attribution = _FIRST_SEEN.get(pid)
        origin = attribution[0] if attribution else "<unknown>"
        lines.append(f"[{worker}] {origin} {pid} {info['name']} {info['cmdline']}\n")

    # Append so multiple xdist workers' reports coexist in one log.
    with open(LEAK_REPORT_LOG, "a") as f:
        f.writelines(lines)

    print(f"\n@@@ CAPSEM PROCESS LEAKS (worker={worker}) @@@", file=sys.stderr)
    for line in lines:
        print(line.rstrip(), file=sys.stderr)
    print(f"({len(lines)} leaked process(es); see {LEAK_REPORT_LOG})", file=sys.stderr)

    session.exitstatus = 1

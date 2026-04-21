"""Root conftest: sys.path wiring + artifact capture for failing tests."""

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


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    outcome = yield
    rep = outcome.get_result()
    if rep.when in ("setup", "call") and rep.failed:
        FAILED_NODEIDS.append(rep.nodeid)

def get_capsem_processes():
    """Get all capsem-* processes."""
    procs = {}
    for proc in psutil.process_iter(['pid', 'name', 'cmdline']):
        try:
            name = proc.info['name'] or ''
            cmdline = proc.info['cmdline'] or []
            # Check both name and cmdline for 'capsem-'
            if 'capsem-' in name or any('capsem-' in arg for arg in cmdline):
                procs[proc.info['pid']] = {
                    'name': name,
                    'cmdline': ' '.join(cmdline)
                }
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
    return procs

@pytest.fixture(scope="session", autouse=True)
def initial_capsem_processes():
    """Snapshot capsem-* processes at session start."""
    # Clear the leak report log at start of session
    if LEAK_REPORT_LOG.exists():
        LEAK_REPORT_LOG.unlink()
    return get_capsem_processes()

@pytest.fixture(autouse=True)
def check_leaks(request, initial_capsem_processes):
    """Check for leaks after each test."""
    nodeid = request.node.nodeid
    before_procs = get_capsem_processes()
    
    yield
    
    after_procs = get_capsem_processes()
    
    for pid, info in after_procs.items():
        if pid not in before_procs and pid not in initial_capsem_processes:
            # Leak detected
            line = f"{nodeid} {pid} {info['name']} {info['cmdline']}\n"
            with open(LEAK_REPORT_LOG, "a") as f:
                f.write(line)

def pytest_sessionfinish(session, exitstatus):
    """Report leaks and fail session if leaks found."""
    if not LEAK_REPORT_LOG.exists():
        return
        
    with open(LEAK_REPORT_LOG, "r") as f:
        lines = f.readlines()
        
    if lines:
        print(f"\n@@@ LEAK DETECTED @@@\n", file=sys.stderr)
        for line in lines:
            print(line.strip(), file=sys.stderr)
        
        # Fail the session if leaks found
        session.exitstatus = 1

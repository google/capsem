"""Root conftest: sys.path wiring + artifact capture for failing tests."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import pytest

# Populated by the hookwrapper below; read by fixtures (ServiceInstance.stop)
# that archive their tmp_dir when this worker session saw any failure.
FAILED_NODEIDS: list[str] = []

# test-artifacts/ at the repo root is the preserve-on-failure destination.
# Gitignored. Fixtures copy their tmp_dir here so service.log /
# sessions/<vm>/process.log / sessions/<vm>/serial.log / session.db all
# survive the normal shutil.rmtree teardown.
ARTIFACTS_ROOT = Path(__file__).parent.parent / "test-artifacts"


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    outcome = yield
    rep = outcome.get_result()
    if rep.when in ("setup", "call") and rep.failed:
        FAILED_NODEIDS.append(rep.nodeid)

"""Fixtures every qualification group shares: the candidate and its session.

A capability group is marked `@pytest.mark.capability(name)` and collected
only for a candidate that declares it, so "not applicable" is a deselection.
Anything that still skips is a required check that did not run, and fails
the qualification (a skip never qualifies an image).
"""

from __future__ import annotations

import pytest
from helpers.image_session import session_of

from tests.ironbank.kingslanding.test_run import service as runtime_service
from tests.qualification.candidate import Candidate, resolve
from tests.qualification.test_agent import model_service

__all__ = ["model_service", "runtime_service"]


@pytest.fixture
def service(request, candidate):
    """Agent startup shares the model fixture; other images use the runtime."""
    fixture = "model_service" if "agent" in candidate.capabilities else "runtime_service"
    return request.getfixturevalue(fixture)


def pytest_configure(config):
    config.addinivalue_line("markers", "capability(name): runs only for a candidate declaring it")


def pytest_collection_modifyitems(config, items):
    marked = [(item, item.get_closest_marker("capability")) for item in items]
    if not any(marker for _, marker in marked):
        return  # the harness's own unit tests need no candidate
    declared = resolve().capabilities
    kept, dropped = [], []
    for item, marker in marked:
        (dropped if marker and marker.args[0] not in declared else kept).append(item)
    if dropped:
        config.hook.pytest_deselected(items=dropped)
        items[:] = kept


def pytest_sessionfinish(session, exitstatus):
    reporter = session.config.pluginmanager.get_plugin("terminalreporter")
    if reporter is not None and reporter.stats.get("skipped") and exitstatus == 0:
        session.exitstatus = pytest.ExitCode.TESTS_FAILED


@pytest.fixture(scope="session")
def candidate() -> Candidate:
    return resolve()


@pytest.fixture
def session(service, candidate, tmp_path):
    """A detached session of the candidate, as a user's create makes one."""
    with session_of(
        service,
        tmp_path / "registry",
        "qualify",
        layout=candidate.layout,
        image_name=candidate.name,
        env={candidate.expect["credential_env"]: "capsem-qualification-test-key"}
        if "agent" in candidate.capabilities
        else None,
    ) as vm_id:
        yield vm_id

"""`just smoke` starts, which it could not.

`SmokeCommand` declared a `Service` resource whose `acquire` unconditionally
raised -- so the public command died on acquisition every time, after the
recipe had already paid for the fast checks and the runtime preparation.

Underneath that was an ownership mistake the raise was standing in for. The
service resolved `CAPSEM_HOME` and `CAPSEM_RUN_DIR` from the ambient
environment when it was *constructed*, so even a working `acquire` would have
started a daemon somewhere other than the workspace beside it -- and stopped
something else on the way out. Bound to the workspace it belongs to, "which
service" has one answer.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import cli  # noqa: F401 - imported so every command registers
from capsem.gate import config as gate_config
from capsem.gate.command import GateCommand
from capsem.gate.errors import GateError
from capsem.gate.lifecycle import held
from capsem.gate.service import Service
from capsem.gate.workspace import Workspace

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: `resources()` takes the runner it should build with; these tests ask
#: *what* is held, so any runner will do.
def _resource_runner():
    from helpers.gate import RecordingRunner

    return RecordingRunner(PROJECT_ROOT)


RUNNER_FOR_RESOURCES = _resource_runner()
CONFIG = gate_config.load(PROJECT_ROOT)


def _smoke(**kwargs):
    return GateCommand.registry["smoke"](
        RecordingRunner(PROJECT_ROOT, **kwargs),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )


def test_the_smoke_command_can_acquire_everything_it_declares() -> None:
    """The failure was total: `acquire` raised on every invocation."""
    command = _smoke()

    for resource in command.resources(RUNNER_FOR_RESOURCES):
        assert resource.acquire is not None
    # The service is the one that used to refuse outright.
    names = [resource.name for resource in command.resources(RUNNER_FOR_RESOURCES)]
    assert names == ["workspace", "service"]


def test_the_service_is_bound_to_the_workspace_beside_it() -> None:
    """Not to whatever `CAPSEM_HOME` happened to say at construction.

    Acquisition order is what makes this expressible: the workspace is taken
    first, so the service can be handed the thing that already exists.
    """
    workspace, service = _smoke().resources(RUNNER_FOR_RESOURCES)

    assert isinstance(workspace, Workspace)
    assert isinstance(service, Service)
    assert service.run_dir == workspace.run_dir


def test_an_ambient_home_cannot_redirect_the_service(monkeypatch) -> None:
    """The mistake the unconditional raise was standing in for."""
    monkeypatch.setenv("CAPSEM_HOME", "/tmp/somebody-elses-capsem")
    monkeypatch.setenv("CAPSEM_RUN_DIR", "/tmp/somebody-elses-capsem/run")

    workspace, service = _smoke().resources(RUNNER_FOR_RESOURCES)

    assert service.run_dir == workspace.run_dir
    assert "somebody-elses" not in str(service.run_dir)


def test_acquiring_the_service_starts_it_and_waits_for_its_socket(
    tmp_path, monkeypatch
) -> None:
    """A pidfile says a process exists; the socket says it is listening."""
    runner = RecordingRunner(PROJECT_ROOT)
    workspace = Workspace(CONFIG)
    service = Service(CONFIG, workspace, runner)
    monkeypatch.setattr(
        "capsem.gate.service._WaitForSocket.perform", lambda self, context: None
    )

    service.acquire()

    assert service.started, "acquiring the service did not start it"
    launched = runner.matching(r"capsem-service")
    assert launched, "no daemon was started; ran:\n  " + "\n  ".join(runner.rendered)
    assert str(workspace.home) in launched[0], (
        "the daemon was started somewhere other than its workspace"
    )


def test_the_service_is_stopped_by_its_own_pidfile(monkeypatch) -> None:
    """Never by pattern: that takes down a developer's installed capsem, or a
    parallel run with a different `CAPSEM_HOME`."""
    stopped: list[Path] = []
    monkeypatch.setattr(
        "capsem.gate.pidfiles.stop_gate_service",
        lambda directory, settings: stopped.append(directory),
    )
    workspace = Workspace(CONFIG)
    service = Service(CONFIG, workspace, RecordingRunner(PROJECT_ROOT))

    service.release()

    assert stopped == [workspace.run_dir]


def test_the_service_is_released_before_its_run_directory_is_removed(
    monkeypatch,
) -> None:
    """Stopping it is what flushes `serial.log`, and that file is what a boot
    failure is argued from.

    Acquisition order gives this for free: the workspace is acquired first, so
    it is released last -- after the service inside it has stopped.
    """
    order: list[str] = []
    command = _smoke()
    workspace, service = command.resources(RUNNER_FOR_RESOURCES)

    monkeypatch.setattr(
        "capsem.gate.pidfiles.stop_gate_service",
        lambda directory, settings: order.append("stop service"),
    )
    monkeypatch.setattr(
        type(workspace), "release", lambda self: order.append("remove run dir")
    )
    monkeypatch.setattr(type(workspace), "acquire", lambda self: None)
    monkeypatch.setattr(type(service), "acquire", lambda self: None)

    with held(workspace, service):
        pass

    assert order == ["stop service", "remove run dir"]


def test_a_failure_preserves_evidence_before_anything_is_released(
    monkeypatch,
) -> None:
    """Release destroys it, so preserve cannot come after."""
    order: list[str] = []
    command = _smoke()
    workspace, service = command.resources(RUNNER_FOR_RESOURCES)

    monkeypatch.setattr(type(workspace), "acquire", lambda self: None)
    monkeypatch.setattr(type(service), "acquire", lambda self: None)
    monkeypatch.setattr(type(service), "release", lambda self: order.append("release"))
    monkeypatch.setattr(
        type(workspace), "preserve", lambda self, error: order.append("preserve")
    )
    monkeypatch.setattr(type(workspace), "release", lambda self: order.append("release"))

    with pytest.raises(GateError), held(workspace, service):
        raise GateError("boom")

    assert order.index("preserve") < order.index("release")

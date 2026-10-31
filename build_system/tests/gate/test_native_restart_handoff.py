"""Private supervisor addresses must not cross the sudo/PAM session boundary."""

import os
import subprocess
import sys
from contextlib import contextmanager

from tests.helpers import run_native_restart, systemd_manager
from tests.helpers.systemd_manager import UserManager


def test_privileged_handoff_preserves_the_callers_bus_not_the_fixture_bus(monkeypatch):
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/caller/runtime")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/caller/runtime/bus")
    private = {
        **os.environ,
        "XDG_RUNTIME_DIR": "/fixture/private-runtime",
        "DBUS_SESSION_BUS_ADDRESS": "unix:path=/fixture/private-runtime/bus",
    }

    @contextmanager
    def manager(*_args, **_kwargs):
        yield UserManager(private, 571)

    calls = []

    def execute(argv, *, env, check):
        calls.append((argv, env))
        return subprocess.CompletedProcess(argv, 0)

    monkeypatch.setattr(run_native_restart, "manager_environment", manager)
    monkeypatch.setattr(run_native_restart.subprocess, "run", execute)
    monkeypatch.setattr(sys, "argv", ["run_native_restart", "--", "probe-command"])
    assert run_native_restart.main() == 0
    argv, environment = calls.pop()
    assert not calls
    assert environment["XDG_RUNTIME_DIR"] == "/caller/runtime"
    assert environment["DBUS_SESSION_BUS_ADDRESS"] == "unix:path=/caller/runtime/bus"
    # Only explicit, authenticated handoff arguments carry the private owner;
    # --inside restores these addresses after sudo and PAM have completed.
    assert argv[argv.index("--manager-runtime") + 1] == private["XDG_RUNTIME_DIR"]
    assert argv[argv.index("--manager-pid") + 1] == "571"


def test_private_manager_binds_its_bus_before_any_units_can_activate(tmp_path, monkeypatch):
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/caller/runtime/bus")
    monkeypatch.delenv("XDG_RUNTIME_DIR", raising=False)
    calls = []

    def execute(argv, **_kwargs):
        calls.append(argv)
        return subprocess.CompletedProcess(argv, 0, stdout="inactive", stderr="")

    monkeypatch.setattr(systemd_manager.subprocess, "run", execute)
    monkeypatch.setattr(systemd_manager, "manager_pid", lambda *_args: 571)
    with systemd_manager.manager_environment(tmp_path, "private-probe", lambda: None) as manager:
        launch = next(argv for argv in calls if "systemd-run" in argv)
        assert f"--setenv=XDG_RUNTIME_DIR={tmp_path / 'systemd-runtime'}" in launch
        assert f"--setenv=DBUS_SESSION_BUS_ADDRESS={manager.environment['DBUS_SESSION_BUS_ADDRESS']}" in launch

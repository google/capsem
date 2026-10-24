"""Exact-owned Tart cleanup errors cannot become successful observations."""

import json
import subprocess

import pytest
from capsem_builder.gate.cancellation import Cancelled

from build_system.tests.packaging.test_macos_tart_glowup import _load_harness

NAME = "capsem-glowup-cleanup-contract"


def adapter(*, running=True, stop=0, delete=0, absent=False):
    calls = []
    present = not absent

    def run(command, **kwargs):
        nonlocal present
        calls.append(command)
        if command[1] == "list":
            rows = [{"Name": NAME, "Source": "local", "Running": running}] if present else []
            return subprocess.CompletedProcess(command, 0, json.dumps(rows), "")
        if command[1] == "stop":
            if isinstance(stop, BaseException):
                raise stop
            return subprocess.CompletedProcess(command, stop, "", "private failure detail")
        if delete == 0:
            present = False
        return subprocess.CompletedProcess(command, delete, "", "private failure detail")

    return calls, run


@pytest.mark.parametrize("kind", ["stop", "delete", "stop_exception"])
def test_refused_cleanup_fails_and_still_attempts_other_actions(kind):
    module = _load_harness()
    calls, run = adapter(
        stop=RuntimeError("private") if kind == "stop_exception" else int(kind == "stop"),
        delete=int(kind == "delete"),
    )
    with pytest.raises(RuntimeError):
        module.cleanup_vm(NAME, run=run)
    assert ["tart", "delete", NAME] in calls
    assert calls[-1] == ["tart", "list", "--format", "json"]


@pytest.mark.parametrize("absent", [False, True])
def test_already_stopped_or_absent_is_proven_not_guessed(absent):
    module = _load_harness()
    calls, run = adapter(running=False, absent=absent)
    module.cleanup_vm(NAME, run=run)
    assert calls[0] == ["tart", "list", "--format", "json"]
    assert ["tart", "stop", NAME] not in calls
    assert (["tart", "delete", NAME] in calls) is not absent


@pytest.mark.parametrize(
    "damage", ["unavailable", "malformed", "foreign", "missing_state", "still_present"]
)
def test_exact_inventory_must_prove_local_identity_and_absence(damage):
    module = _load_harness()
    calls = []

    def run(command, **kwargs):
        calls.append(command)
        row = {"Name": NAME, "Source": "oci" if damage == "foreign" else "local", "Running": False}
        if damage == "missing_state":
            del row["Running"]
        return subprocess.CompletedProcess(
            command,
            int(damage == "unavailable"),
            "{}" if damage == "malformed" else json.dumps([row]),
            "private",
        )

    with pytest.raises(RuntimeError):
        module.cleanup_vm(NAME, run=run)
    if damage != "still_present":
        assert len(calls) == 1


@pytest.mark.parametrize("primary", [False, True])
def test_all_final_actions_run_and_original_failure_is_preserved(primary, capsys):
    module = _load_harness()
    calls = []

    def action(label):
        def call():
            calls.append(label)
            raise RuntimeError("private secret subprocess output")

        return call

    original = ValueError("original installation failure") if primary else None
    kwargs = {
        "primary_error": original,
        "diagnostics": action("diagnostics"),
        "vm": action("VM"),
        "runner": action("runner"),
        "cache": action("cache"),
    }
    if original is not None:
        module.finish_cleanup(**kwargs)
        assert str(original) == "original installation failure"
        assert "Tart final cleanup failed" in original.__notes__[0]
    else:
        with pytest.raises(RuntimeError, match="Tart final cleanup failed"):
            module.finish_cleanup(**kwargs)
    assert calls == (["diagnostics"] if primary else []) + ["VM", "runner", "cache"]
    assert "secret" not in capsys.readouterr().err


@pytest.mark.parametrize("cancellation", [KeyboardInterrupt, Cancelled])
def test_cleanup_defers_cancellation_until_other_owned_resources_are_settled(cancellation):
    module = _load_harness()
    calls = []

    def cancel():
        raise cancellation("cancelled")

    with pytest.raises(cancellation):
        module.finish_cleanup(
            primary_error=None,
            diagnostics=None,
            vm=cancel,
            runner=lambda: calls.append("runner"),
            cache=lambda: calls.append("cache"),
        )
    assert calls == ["runner", "cache"]


def test_cleanup_refuses_to_stop_or_delete_foreign_tart_vms() -> None:
    module = _load_harness()
    calls: list[list[str]] = []

    def record(command: list[str], **_: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        if command[1] == "list":
            rows = (
                []
                if any(call[1] == "delete" for call in calls)
                else [{"Name": "capsem-glowup-123", "Source": "local", "Running": True}]
            )
            return subprocess.CompletedProcess(command, 0, json.dumps(rows), "")
        return subprocess.CompletedProcess(command, 0, "", "")

    with pytest.raises(ValueError, match="owned VM name"):
        module.cleanup_vm("developer-workstation", run=record)

    module.cleanup_vm("capsem-glowup-123", run=record)
    assert calls == [
        ["tart", "list", "--format", "json"],
        ["tart", "stop", "capsem-glowup-123"],
        ["tart", "delete", "capsem-glowup-123"],
        ["tart", "list", "--format", "json"],
    ]

"""Final settlement of the existing exact-owned disposable Tart lifecycle."""

from __future__ import annotations

import json
import subprocess
import sys
from collections.abc import Callable

from capsem_builder.cache.runtimemodels import TartRuntimePolicy
from capsem_builder.gate.cancellation import Cancelled


def remove_vm(vm_name: str, policy: TartRuntimePolicy, *, run) -> None:
    if not vm_name.startswith(policy.vm_prefixes[0]):
        raise ValueError("refusing non-owned VM name")

    def state() -> bool | None:
        result = run(
            [policy.command, "list", "--format", "json"],
            check=False,
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
            timeout=policy.timeout_seconds,
        )
        if result.returncode != 0:
            raise RuntimeError("Tart cleanup inventory failed")
        try:
            rows = json.loads(result.stdout)
            if not isinstance(rows, list) or any(
                not isinstance(row, dict) or not isinstance(row.get("Name"), str) for row in rows
            ):
                raise ValueError("invalid inventory")
            selected = [row for row in rows if row["Name"] == vm_name]
            if not selected:
                return None
            if (
                len(selected) != 1
                or selected[0].get("Source") != "local"
                or type(selected[0].get("Running")) is not bool
            ):
                raise ValueError("ambiguous VM identity/state")
            return selected[0]["Running"]
        except (TypeError, ValueError) as error:
            raise RuntimeError("Tart cleanup inventory cannot prove exact VM state") from error

    running = state()
    if running is None:
        return
    failures: list[BaseException] = []
    for action in ("stop", "delete") if running else ("delete",):
        try:
            result = run(
                [policy.command, action, vm_name],
                check=False,
                capture_output=True,
                text=True,
                stdin=subprocess.DEVNULL,
                timeout=policy.mutation_timeout_seconds,
            )
            if result.returncode != 0:
                raise RuntimeError(f"Tart cleanup {action} failed (status {result.returncode})")
        except BaseException as error:
            failures.append(error)
    try:
        if state() is not None:
            raise RuntimeError("Tart cleanup left the exact owned VM present")
    except BaseException as error:
        failures.append(error)
    if failures:
        for error in failures:
            if isinstance(error, Cancelled) or not isinstance(error, Exception):
                raise error
        raise RuntimeError("Tart cleanup did not settle the exact owned VM") from failures[0]


def finish_cleanup(
    *,
    primary_error: BaseException | None,
    diagnostics: Callable[[], object] | None,
    vm: Callable[[], object],
    runner: Callable[[], object],
    cache: Callable[[], object],
) -> None:
    failures: list[tuple[str, BaseException]] = []
    actions = (("diagnostics", diagnostics), ("VM", vm), ("runner", runner), ("cache", cache))
    for name, action in actions:
        if action is None or (name == "diagnostics" and primary_error is None):
            continue
        try:
            action()
        except BaseException as error:
            failures.append((name, error))
    if not failures:
        return
    message = "Tart final cleanup failed: " + ", ".join(
        f"{name} ({type(error).__name__})" for name, error in failures
    )
    # Preserve the actual original failure, while retaining a bounded sanitized
    # cleanup diagnosis. Never leak subprocess output or replace that failure.
    if primary_error is not None:
        primary_error.add_note(message)
        print(message, file=sys.stderr)
        return
    for _, error in failures:
        if isinstance(error, Cancelled) or not isinstance(error, Exception):
            raise error
    raise RuntimeError(message) from failures[0][1]

"""Exact VM selections and approved startup timing, read before a mutation."""
from __future__ import annotations

import re
from uuid import UUID

from . import _operations as api
from . import models
from ._transport import HttpError, Transport
from .execution import CREATE_READY_SECS, command_deadline

VM_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}")


def _valid_uuid(value: str) -> bool:
    parsed = UUID(value)
    return parsed.int != 0 and str(parsed) == value


def _validate_startup(seconds: int) -> None:
    if seconds > 2**32 - 1:
        raise ValueError("readiness timeout exceeds the approved domain")


def startup_timeout(transport: Transport, seconds: int) -> float:
    _validate_startup(seconds)
    return command_deadline(transport.timeout, CREATE_READY_SECS + seconds)


async def lifecycle_readiness(
    transport: Transport, vm_id: str, intent: models.VmLifecycleIntent,
    *, expected: models.VmLifecycleSelection | None = None,
) -> models.VmReadinessResponse:
    if not isinstance(vm_id, str) or not VM_NAME.fullmatch(vm_id):
        raise ValueError("invalid VM identifier")
    changed = '{"error":"lifecycle selection changed; request a new readiness view"}'
    try:
        readiness = await api.get_vm_readiness(transport, id=vm_id, intent=intent)
    except HttpError as error:
        if expected is not None and error.status == 404:
            raise HttpError(409, changed) from error
        raise
    selection = readiness.selection
    if intent is not models.VmLifecycleIntent.CLONE and selection.id != vm_id:
        raise ValueError("lifecycle readiness targets a different VM")
    if not VM_NAME.fullmatch(selection.id):
        raise ValueError("invalid VM identifier in lifecycle readiness")
    if selection.generation is None:
        if selection.running:
            raise ValueError("running lifecycle selection requires its original generation")
    elif not _valid_uuid(selection.generation):
        raise ValueError("invalid lifecycle generation")
    environment = selection.environment
    if environment is not None and not (
        _valid_uuid(environment.environment_id) and _valid_uuid(environment.revision_id)
    ):
        raise ValueError("invalid lifecycle environment reference")
    _validate_startup(readiness.startup_timeout_seconds)
    if expected is not None and selection != expected:
        raise HttpError(409, changed)
    return readiness


async def prepare_start(transport: Transport, vm_id: str) -> tuple[models.VmStartRequest, float]:
    readiness = await lifecycle_readiness(transport, vm_id, models.VmLifecycleIntent.START)
    selected = readiness.selection
    seconds = readiness.startup_timeout_seconds if selected.environment is not None else 0
    return models.VmStartRequest(expected=selected), startup_timeout(transport, seconds)


async def prepare_creation(transport: Transport, request: models.ProvisionRequest) -> float:
    seconds = 0
    if request.from_ is not None:
        readiness = await lifecycle_readiness(
            transport, request.from_, models.VmLifecycleIntent.CLONE, expected=request.expected_source,
        )
        request.expected_source = readiness.selection
        if request.container is None and readiness.selection.environment is not None:
            seconds = readiness.startup_timeout_seconds
    if request.environment is not None and not request.attach:
        selected = request.environment
        response = await api.get_environment_readiness(
            transport, id=selected.environment_id, revision_id=selected.revision_id)
        if response.environment != selected:
            raise ValueError("environment readiness identity changed")
        seconds = response.startup_timeout_seconds
    return startup_timeout(transport, 0 if request.attach else seconds)

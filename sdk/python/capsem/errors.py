"""Structured exceptions and error mapping for the Capsem gateway SDK."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import TYPE_CHECKING, Self

from . import models
from ._transport import CapsemError, HttpError, Transport
from .execution import CREATE_READY_SECS, EXEC_TIMEOUT_CEILING_SECS, ExecResult

if TYPE_CHECKING:
    from .vm import VM

__all__ = [
    "CapsemError", "CapsemTimeoutError", "CreateTimeoutError",
    "ExecTimeoutError", "HttpError", "VmNotFoundError",
]


class CapsemTimeoutError(TimeoutError, CapsemError):
    """Base exception for Capsem SDK operation timeouts."""

    status: int | None
    body: str | None
    vm_id: str | None
    vm_name: str | None
    vm: VM | None
    command: str | None
    timeout_secs: int | None
    deadline_secs: float | None

    def __init__(
        self, message: str, *, vm_id: str | None = None, vm_name: str | None = None,
        vm: VM | None = None, command: str | None = None, timeout_secs: int | None = None,
        deadline_secs: float | None = None, status: int | None = None, body: str | None = None,
    ) -> None:
        self.vm_id = vm_id
        self.vm_name = vm_name
        self.vm = vm
        self.command = command
        self.timeout_secs = timeout_secs
        self.deadline_secs = deadline_secs
        self.status = status
        self.body = body
        TimeoutError.__init__(self, message)


class CreateTimeoutError(CapsemTimeoutError):
    """Raised when `Hypervisor.create` crosses the HTTP 504 or client deadline."""


class ExecTimeoutError(CapsemTimeoutError):
    """Raised when `VM.exec` or `Hypervisor.run` times out on the service or client."""


class VmNotFoundError(HttpError, LookupError):
    """Raised when a VM handle cannot be resolved by name or ID (HTTP 404)."""

    vm_id: str | None
    vm_name: str | None

    def __init__(self, body: str, *, vm_id: str | None = None, vm_name: str | None = None, status: int = 404) -> None:
        self.vm_id = vm_id
        self.vm_name = vm_name
        HttpError.__init__(self, status, body)

    def __reduce__(self) -> tuple[type[Self], tuple[object, ...], dict[str, object]]:
        return (type(self), (self.body,), self.__dict__.copy())


@dataclass(frozen=True)
class _ErrorContext:
    vm_id: str | None = None
    vm_name: str | None = None
    bind_vm: Callable[[str | None], VM | None] | None = None
    command: str | None = None
    timeout_secs: int | None = None
    deadline_secs: float | None = None


_ERROR_DISPATCH: dict[models.ErrorCode, Callable[[HttpError, _ErrorContext], CapsemError]] = {
    models.ErrorCode.VM_NOT_FOUND: lambda error, ctx: VmNotFoundError(
        error.body,
        vm_id=error.response.vm_id if error.response and error.response.vm_id else ctx.vm_id,
        vm_name=ctx.vm_name, status=error.status,
    ),
    models.ErrorCode.CREATE_TIMEOUT: lambda error, ctx: CreateTimeoutError(
        f"HTTP {error.status}: {error.body}",
        vm_id=error.response.vm_id if error.response else None,
        vm_name=ctx.vm_name,
        vm=ctx.bind_vm(error.response.vm_id if error.response else None) if ctx.bind_vm is not None else None,
        deadline_secs=ctx.deadline_secs, status=error.status, body=error.body,
    ),
    models.ErrorCode.EXEC_TIMEOUT: lambda error, ctx: ExecTimeoutError(
        f"HTTP {error.status}: {error.body}",
        vm_id=ctx.vm_id, command=ctx.command,
        timeout_secs=(
            error.response.timeout_secs
            if error.response and error.response.timeout_secs is not None
            else ctx.timeout_secs
        ),
        status=error.status, body=error.body,
    ),
}


def _map_http_error(
    error: HttpError, ctx: _ErrorContext, *, fallback_code: models.ErrorCode | None = None,
) -> CapsemError | None:
    code = error.code or fallback_code
    if code is None:
        return None
    builder = _ERROR_DISPATCH.get(code)
    return builder(error, ctx) if builder is not None else None


async def _create_vm_with_timeout(
    op: Awaitable[models.ProvisionResponse], transport: Transport, *,
    bind_vm: Callable[..., VM], name: str, container: bool, request_timeout: float,
) -> VM:
    vm_name = name or None

    def _bind(vm_id: str | None) -> VM | None:
        if vm_id is None and vm_name is None:
            return None
        return bind_vm(transport, id=vm_id, name=vm_name, container=container)

    ctx = _ErrorContext(vm_name=vm_name, bind_vm=_bind, deadline_secs=float(CREATE_READY_SECS))
    try:
        response = await op
    except HttpError as error:
        fallback = models.ErrorCode.CREATE_TIMEOUT if error.status == 504 else None
        mapped = _map_http_error(error, ctx, fallback_code=fallback)
        if mapped is not None:
            raise mapped from error
        raise
    except TimeoutError as error:
        raise CreateTimeoutError(
            str(error) or "create timed out",
            vm_id=None, vm_name=vm_name, vm=_bind(None), deadline_secs=float(request_timeout),
        ) from error
    return bind_vm(transport, id=response.id, name=response.name, container=container)


async def _exec_with_timeout(
    op: Awaitable[models.ExecResponse], *, command: str, timeout_secs: int | None,
    vm_id: str | None = None, vm_name: str | None = None,
) -> ExecResult:
    effective_secs = (
        EXEC_TIMEOUT_CEILING_SECS if timeout_secs is None else min(timeout_secs, EXEC_TIMEOUT_CEILING_SECS)
    )
    ctx = _ErrorContext(vm_id=vm_id, vm_name=vm_name, command=command, timeout_secs=effective_secs)
    try:
        response = await op
    except HttpError as error:
        fallback = models.ErrorCode.EXEC_TIMEOUT if error.status in (408, 504) else None
        mapped = _map_http_error(error, ctx, fallback_code=fallback)
        if mapped is not None:
            raise mapped from error
        raise
    except TimeoutError as error:
        raise ExecTimeoutError(
            str(error) or "command timed out",
            vm_id=vm_id, command=command, timeout_secs=effective_secs,
        ) from error
    return ExecResult.from_wire(response)

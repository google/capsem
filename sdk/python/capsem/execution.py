"""Helpers for the byte-safe command execution contract."""

from __future__ import annotations

from base64 import b64decode

from .models import ExecOutput, ExecOutputEncoding, ExecResponse

#: The service's ceiling for one exec or run, which is also its default.
EXEC_TIMEOUT_CEILING_SECS = 60 * 60
#: The gateway's budget for readiness, boot and teardown around a command.
GATEWAY_REQUEST_BUDGET_SECS = 120


def command_deadline(default: float, timeout_secs: int | None) -> float:
    """HTTP deadline for exec or run: the service answers only when the
    command ends, so wait at least as long as the gateway does for it."""
    command = EXEC_TIMEOUT_CEILING_SECS if timeout_secs is None else timeout_secs
    return max(default, command + GATEWAY_REQUEST_BUDGET_SECS)


def decode_exec_output(output: ExecOutput) -> bytes:
    """Decode one stdout or stderr value to its exact bytes."""
    if output.encoding is ExecOutputEncoding.UTF8:
        return output.data.encode()
    return b64decode(output.data, validate=True)


class ExecResult(ExecResponse):
    """Command result with exact byte access and a useful string form."""

    @property
    def stdout_bytes(self) -> bytes:
        return decode_exec_output(self.stdout)

    @property
    def stderr_bytes(self) -> bytes:
        return decode_exec_output(self.stderr)

    def __str__(self) -> str:
        return self.stdout_bytes.decode("utf-8", errors="backslashreplace").removesuffix("\n")

    @classmethod
    def from_wire(cls, response: ExecResponse) -> ExecResult:
        return cls.model_validate(response.model_dump(exclude_none=True))

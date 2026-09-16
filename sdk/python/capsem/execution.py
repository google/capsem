"""Helpers for the byte-safe command execution contract."""

from __future__ import annotations

from base64 import b64decode

from .models import ExecOutput, ExecOutputEncoding, ExecResponse


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

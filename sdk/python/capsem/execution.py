"""Helpers for the byte-safe command execution contract."""

from __future__ import annotations

from base64 import b64decode

from .models import ExecOutput, ExecOutputEncoding


def decode_exec_output(output: ExecOutput) -> bytes:
    """Decode one stdout or stderr value to its exact bytes."""
    if output.encoding is ExecOutputEncoding.UTF8:
        return output.data.encode()
    return b64decode(output.data, validate=True)

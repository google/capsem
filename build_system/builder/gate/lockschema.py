"""Schema for the user-scoped machine gate lock."""

from __future__ import annotations

from pathlib import PurePosixPath

from pydantic import field_validator

from .configschema import Strict


class LockConfig(Strict):
    """One holder at a time, proven by the kernel rather than by a PID file."""

    path: str
    holder_record: str
    report_after_seconds: float
    wait_timeout_seconds: float
    poll_interval_seconds: float
    run_marker: str

    @field_validator("path", "holder_record")
    @classmethod
    def _must_be_user_scoped(cls, value: str) -> str:
        parts = PurePosixPath(value).parts
        if (len(parts) > 1 and parts[0] == "~") or PurePosixPath(value).is_absolute():
            return value
        raise ValueError("machine lock paths must be absolute or user-home-relative")


class BoundedLeaseConfig(Strict):
    """Which direct commands are machine work, and so take the gate's lock."""

    programs: tuple[str, ...]
    command_prefixes: tuple[tuple[str, ...], ...]
    wrappers: tuple[str, ...]
    exempt_subcommands: tuple[str, ...]
    wait_exit_code: int

    @field_validator("command_prefixes")
    @classmethod
    def _nonempty_prefixes(cls, value: tuple[tuple[str, ...], ...]) -> tuple[tuple[str, ...], ...]:
        if any(not prefix or any(not token for token in prefix) for prefix in value):
            raise ValueError("machine-work command prefixes must contain nonempty tokens")
        return value


class LocksConfig(Strict):
    gate: LockConfig
    bounded: BoundedLeaseConfig

"""Config-owned deadline for one foreground command."""

from __future__ import annotations

import time
from dataclasses import dataclass

from .errors import GateError


@dataclass(frozen=True)
class Deadline:
    """An optional command bound that also caps cancellation polling."""

    seconds: float | None
    expires_at: float | None

    @classmethod
    def start(cls, seconds: float | None) -> Deadline:
        if seconds is None:
            return cls(None, None)
        if seconds <= 0:
            raise ValueError("foreground command timeout must be positive")
        return cls(seconds, time.monotonic() + seconds)

    def poll_seconds(self, default: float) -> float:
        self.check()
        if self.expires_at is None:
            return default
        return min(default, max(self.expires_at - time.monotonic(), 0.001))

    def check(self) -> None:
        if self.expires_at is not None and time.monotonic() >= self.expires_at:
            assert self.seconds is not None
            raise GateError(f"foreground command timed out after {self.seconds:g}s")

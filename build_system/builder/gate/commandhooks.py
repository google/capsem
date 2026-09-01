"""Optional lifecycle hooks kept outside the fixed-size execution funnel."""

from __future__ import annotations

from .sourcecommit import SourceCommit


class CommandHooks:
    """No-op defaults overridden only by commands with admission state."""

    def admit(self, commit: SourceCommit | None) -> None:
        """Apply command-specific pre-run admission after all re-execs."""
        del commit

    def completed(self, commit: SourceCommit | None) -> None:
        """Record command-specific success after the plan returns."""
        del commit

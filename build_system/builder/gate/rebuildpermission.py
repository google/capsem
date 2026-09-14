"""Whether a run may pay for a rebuild whose inputs changed: the `--slow` flag."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class RebuildPermission:
    """Whether a run may rebuild host assets whose inputs changed.

    Missing assets are always rebuilt: there is nothing cheaper to do. So are
    assets stale only through source crates: a change to capsem-core must
    rebuild to be tested at all, so refusing it offers no decision. What is
    refused without `--slow` is a change to an expensive input -- a lock, the
    toolchain, a builder Dockerfile, the kernel defconfig -- where honouring
    it costs the guest Rust builder image, every guest agent, the initrd, the
    images and the host binaries, and where the change is usually an accident.
    The refusal names the input so that cost is chosen rather than discovered
    twenty minutes in. `[assets] expensive_inputs` is the list.
    """

    slow: bool = False

    @classmethod
    def from_args(cls, args: object) -> RebuildPermission:
        return cls(bool(getattr(args, "slow", False)))


#: What a run gets when it did not ask: rebuild missing assets, refuse stale ones.
DEFAULT_PERMISSION = RebuildPermission()


class ExpensiveRebuildRefused(RuntimeError):
    """Stale host assets and no `--slow`: the run stops before rebuilding."""

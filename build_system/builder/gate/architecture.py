"""The typed architecture vocabulary shared by the gate and bootstrap tools."""

from __future__ import annotations

from enum import Enum, auto


class Arch(Enum):
    """Which architecture work belongs to, without duplicating config spellings.

    `ANY` is architecture-neutral and `HOST` is specifically native work. The
    concrete member names are held equal to `[architectures]` by Citadel.
    """

    HOST = auto()
    X86_64 = auto()
    ARM64 = auto()
    ANY = auto()

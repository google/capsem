"""What machine the gate is running on.

Runtime facts, not configuration: nobody chooses these, and a checkout cannot
declare them. Kept apart from `config` so the difference stays obvious, and
gathered here rather than spelled as `uname -s` in five recipes -- each of
which was its own chance to spell the answer differently.
"""

from __future__ import annotations

import os
import platform
from pathlib import Path


def machine() -> str:
    """The processor architecture, as the kernel reports it."""
    return platform.machine()


def system() -> str:
    """The operating system: `Darwin` or `Linux`."""
    return platform.system()


def on_macos() -> bool:
    return system() == "Darwin"


def on_linux() -> bool:
    return system() == "Linux"


def device_available(path: str) -> bool:
    """Whether a device node exists and this process may read and write it."""
    return os.access(path, os.R_OK | os.W_OK)


def user() -> tuple[int, int]:
    """The uid and gid that must own anything a container writes to the mount."""
    return os.getuid(), os.getgid()


def home() -> Path:
    """The user's home directory, read once through here like every other
    runtime fact so a test can move it."""
    return Path.home()

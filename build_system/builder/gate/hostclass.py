"""Which hardware a pytest step measures on, for benchmark evidence."""

from __future__ import annotations

import os
from collections.abc import Mapping
from enum import StrEnum

from .config import GateConfig


class HostClass(StrEnum):
    LOCAL = "local"
    HOSTED = "hosted"


def of(config: GateConfig, environment: Mapping[str, str] | None = None) -> HostClass:
    """Hosted only on the exact runner signal `[benchmark_regression]` names.

    Read here, in the gate's own process, and handed to pytest explicitly. The
    test process cannot be trusted to find out for itself: tests/conftest.py
    strips the release variables, and the timing ratchet once compared a
    GitHub-hosted runner with the build box's floor because nothing told it
    which machine it was on.
    """
    source = os.environ if environment is None else environment
    hosted = config.benchmark_regression.hosted_environment
    if hosted and all(source.get(name) == value for name, value in hosted.items()):
        return HostClass.HOSTED
    return HostClass.LOCAL

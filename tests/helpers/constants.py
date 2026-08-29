"""Shared constants for integration tests.

Single source of truth for VM resources, timeouts, and other values
used across capsem-mcp and capsem-service test suites.
"""

import os
from collections.abc import Mapping
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ASSETS_VARIABLE = "CAPSEM_ASSETS_DIR"
PROFILES_VARIABLE = "CAPSEM_PROFILES_DIR"
BIN_VARIABLE = "CAPSEM_RELEASE_BIN_DIR"


def content_assets_root(environment: Mapping[str, str] | None = None) -> Path:
    """The manifest-selected asset root for this test process."""
    source = os.environ if environment is None else environment
    return Path(source.get(ASSETS_VARIABLE) or PROJECT_ROOT / "target" / "assets")


def content_profiles_root(environment: Mapping[str, str] | None = None) -> Path:
    """The profile catalog paired with the selected asset manifest."""
    source = os.environ if environment is None else environment
    return Path(source.get(PROFILES_VARIABLE) or PROJECT_ROOT / "target" / "config" / "profiles")


def host_bin_root(environment: Mapping[str, str] | None = None) -> Path:
    """Where this test process's host binaries are, built or pulled.

    The third of these, and for the same reason as the first two: a release
    lane qualifies from a private prefix carrying only tracked files, so
    `target/debug` is a directory nothing ever wrote. The gate already answers
    this question -- `qualification.py` resolves `CAPSEM_RELEASE_BIN_DIR` or
    falls back to `target/debug` -- and helpers that spelled the fallback out
    for themselves were reading past a lane that had told them otherwise.

    That cost the tenth binary-release dispatch, which died on
    `target/debug/capsem-service` after every other job in the run had passed.
    """
    source = os.environ if environment is None else environment
    return Path(source.get(BIN_VARIABLE) or PROJECT_ROOT / "target" / "debug")


ASSETS_DIR = content_assets_root()
PROFILES_DIR = content_profiles_root()
BIN_DIR = host_bin_root()

# Default VM resources
DEFAULT_RAM_MB = 2048
DEFAULT_CPUS = 2
# Release CI runs the complete VM suite once for every selected manifest
# profile. Local/default test behavior remains the canonical code profile.
CODE_PROFILE_ID = os.environ.get("CAPSEM_TEST_PROFILE", "code")

# Timeouts (seconds)
EXEC_READY_TIMEOUT = 60  # Max seconds to wait for a VM to become exec-ready
EXEC_TIMEOUT_SECS = 60  # Per-command execution timeout passed to the server
HTTP_TIMEOUT = 90  # HTTP request timeout for long-running operations (e.g. boot)

# Guest filesystem paths
# The workspace root inside the guest VM -- file I/O is restricted to this directory.
GUEST_WORKSPACE = "/root"

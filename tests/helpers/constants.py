"""Shared constants for integration tests.

Single source of truth for VM resources, timeouts, and other values
used across capsem-mcp and capsem-service test suites.
"""

import os

# Default VM resources
DEFAULT_RAM_MB = 2048
DEFAULT_CPUS = 2
# Release CI runs the complete VM suite once for every selected manifest
# profile. Local/default test behavior remains the canonical code profile.
CODE_PROFILE_ID = os.environ.get("CAPSEM_TEST_PROFILE", "code")

# Timeouts (seconds)
EXEC_READY_TIMEOUT = 60    # Max seconds to wait for a VM to become exec-ready
EXEC_TIMEOUT_SECS = 60     # Per-command execution timeout passed to the server
HTTP_TIMEOUT = 90           # HTTP request timeout for long-running operations (e.g. boot)

# Guest filesystem paths
# The workspace root inside the guest VM -- file I/O is restricted to this directory.
GUEST_WORKSPACE = "/root"

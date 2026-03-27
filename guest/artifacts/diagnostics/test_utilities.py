"""Unix utility availability tests."""

import shutil

import pytest


@pytest.mark.parametrize("util", [
    # System inspection
    "df", "ps", "free", "lsof", "find", "grep", "sed", "awk",
    "less", "file", "tar", "strace", "lsblk", "mount", "id",
    "hostname", "uname", "uptime", "dmesg", "vim", "du",
    # Core file operations
    "cat", "cp", "mv", "rm", "mkdir", "chmod", "touch", "ln",
    # Text processing
    "sort", "uniq", "wc", "cut", "tr", "diff", "tee", "xargs",
    # Network and shell
    "curl", "ip", "bash", "env",
    # Benchmarks
    "capsem-bench",
])
def test_utility_available(util):
    """Each required unix utility must be in PATH."""
    assert shutil.which(util) is not None, f"{util} not found in PATH"

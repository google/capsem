import os
import subprocess
import pathlib

import pytest

TESTS_OUTPUT_DIR = pathlib.Path("/root/tests")


def pytest_ignore_collect(collection_path, config):
    """Cleanly ignore this directory if not running inside the capsem VM."""
    if os.geteuid() != 0 or not os.access("/root", os.W_OK):
        return True
    return False


@pytest.fixture(autouse=True)
def ensure_output_dir():
    """Create output directory for test artifacts."""
    TESTS_OUTPUT_DIR.mkdir(parents=True, exist_ok=True)


@pytest.fixture
def output_dir():
    """Return the shared output directory path."""
    return TESTS_OUTPUT_DIR


def run(cmd, timeout=10):
    """Run a shell command and return CompletedProcess."""
    return subprocess.run(
        cmd, shell=True, capture_output=True, text=True, timeout=timeout
    )

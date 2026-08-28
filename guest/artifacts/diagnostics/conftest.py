import os

import pytest

from .diagnostic_support import TESTS_OUTPUT_DIR


def pytest_ignore_collect(collection_path, config):
    """Cleanly ignore this directory if not running inside the capsem VM."""
    return bool(os.geteuid() != 0 or not os.access("/root", os.W_OK))


@pytest.fixture(autouse=True)
def ensure_output_dir():
    """Create output directory for test artifacts."""
    TESTS_OUTPUT_DIR.mkdir(parents=True, exist_ok=True)


@pytest.fixture
def output_dir():
    """Return the shared output directory path."""
    return TESTS_OUTPUT_DIR

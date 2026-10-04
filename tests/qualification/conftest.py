"""Fixtures every qualification group shares: the candidate and its session."""

from __future__ import annotations

import pytest
from helpers.image_session import session_of

from tests.ironbank.kingslanding.test_run import service
from tests.qualification.candidate import Candidate, resolve

__all__ = ["service"]


@pytest.fixture(scope="session")
def candidate() -> Candidate:
    return resolve()


@pytest.fixture
def session(service, candidate, tmp_path):
    """A detached session of the candidate, as a user's create makes one."""
    with session_of(
        service, tmp_path / "registry", "qualify", layout=candidate.layout, image_name=candidate.name
    ) as vm_id:
        yield vm_id

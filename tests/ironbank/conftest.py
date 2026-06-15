"""Shared Ironbank fixtures."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
from types import ModuleType
from typing import Iterator

import pytest


def _load_model_client_contract_module() -> ModuleType:
    module_path = Path(__file__).with_name("test_model_client_ledger_contract.py")
    spec = importlib.util.spec_from_file_location(
        "ironbank_model_client_ledger_contract",
        module_path,
    )
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


_MODEL_CLIENT_CONTRACT = _load_model_client_contract_module()


@pytest.fixture
def model_client_env() -> Iterator[object]:
    yield from _MODEL_CLIENT_CONTRACT.model_client_env.__wrapped__()

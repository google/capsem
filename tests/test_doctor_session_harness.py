"""Tests for the doctor session validation harness."""

from pathlib import Path

import importlib.util


MODULE_PATH = Path(__file__).resolve().parents[1] / "scripts" / "doctor_session_test.py"
spec = importlib.util.spec_from_file_location("doctor_session_test", MODULE_PATH)
doctor_session_test = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(doctor_session_test)


def test_capsem_run_session_names_accept_current_and_legacy_shapes():
    assert doctor_session_test._is_capsem_run_session_name("fancy-narwhal-tmp")
    assert doctor_session_test._is_capsem_run_session_name("run-20260529-abc")
    assert not doctor_session_test._is_capsem_run_session_name("fancy-narwhal-tmp-failed")
    assert not doctor_session_test._is_capsem_run_session_name("persistent-vm")

"""A failed pytest step's summary names the cause, not the log's last lines."""

from __future__ import annotations

from capsem_builder.gate import pytestexcerpt

CAUSE = (
    'E   AssertionError: time="2026-09-11T11:54:41-04:00" level=error msg="runc run failed: '
    'error running hook: exit status 1, stderr: Error: Unknown device type."'
)

LOG = f"""\
============================= test session starts ==============================
collected 3 items

tests/ironbank/kingslanding/test_run.py::test_ok PASSED                  [ 33%]
tests/ironbank/kingslanding/test_publish.py::test_a ERROR                [ 66%]
tests/ironbank/kingslanding/test_egress.py::test_b FAILED                [100%]

==================================== ERRORS ====================================
______________________________ ERROR at setup of test_a ______________________________
tests/ironbank/kingslanding/test_publish.py:45: in ready
    assert process.poll() is None, (tmp_path / "stderr").read_text()
{CAUSE}
E     Image sha256:6d8f0d0a45f3557fdabea4a3237e10e2bbc0ee71789df9cafdcf3ad866e4c50e
E     Running redis (7bc9a68d-5c23-40bc-a869-04197b402dde)
E     Published 127.0.0.1:50525 -> 6379/tcp (router 82518)
E   assert 1 is None
=================================== FAILURES ===================================
_______________________________ test_b _______________________________
tests/ironbank/kingslanding/test_egress.py:121: in test_b
    assert results.get("http") == "0", run.stdout + run.stderr
{CAUSE}
E   assert None == '0'
=========================== short test summary info ============================
ERROR tests/ironbank/kingslanding/test_publish.py::test_a - AssertionError: time=...
FAILED tests/ironbank/kingslanding/test_egress.py::test_b - AssertionError: time=...
========================= 1 failed, 1 passed, 1 error in 40.16s =========================
"""


def test_the_summary_leads_and_each_block_opens_with_its_cause() -> None:
    text = pytestexcerpt.excerpt(LOG.splitlines(), per_failure=2)
    lines = text.splitlines()
    assert lines[0] == "short test summary:"
    assert lines[1].strip().startswith("ERROR tests/ironbank/kingslanding/test_publish.py::test_a")
    assert lines[2].strip().startswith("FAILED tests/ironbank/kingslanding/test_egress.py::test_b")
    assert "ERROR at setup of test_a:" in lines
    assert "test_b:" in lines
    # The cause is the first line of every block, whatever follows it.
    assert text.count("Unknown device type") == 2
    assert "Published 127.0.0.1" not in text, "per_failure caps the lines kept per block"
    assert "collected 3 items" not in text and "PASSED" not in text


def test_a_log_that_is_not_pytest_output_yields_nothing() -> None:
    assert pytestexcerpt.excerpt(["cargo build", "error: could not compile"], per_failure=4) == ""
    assert pytestexcerpt.excerpt(LOG.splitlines(), per_failure=0) == ""


def test_blocks_without_a_message_are_not_listed() -> None:
    log = LOG.replace(CAUSE, "").replace("E   assert None == '0'", "").replace("E   assert 1 is None", "")
    log = "\n".join(line for line in log.splitlines() if not line.startswith("E "))
    text = pytestexcerpt.excerpt(log.splitlines(), per_failure=3)
    assert text.startswith("short test summary:")
    assert "test_b:" not in text

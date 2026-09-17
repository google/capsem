"""Every generated operation forwards a per-call HTTP deadline.

Exec and run answer only when the command finishes, which the service
bounds by the request's `timeout_secs` (default one hour). A client whose
only deadline is the transport default gave up on those calls after 30 s
while the command kept running, and retrying agents started it again. The
facades stretch the deadline per call, so the generated layer must carry it.
"""

from __future__ import annotations

from pathlib import Path

from capsem_builder.sdkgen.operations import read_operations
from capsem_builder.sdkgen.python_operations import render_operations as render_python
from capsem_builder.sdkgen.typescript_operations import render_operations as render_typescript

SPEC = Path(__file__).resolve().parents[3] / "sdk/specification/openapi.json"


def test_python_operations_accept_and_forward_a_request_timeout() -> None:
    files = render_python(read_operations(SPEC))
    operations = {name: text for name, text in files.items() if name != "__init__.py"}
    assert operations
    for name, text in operations.items():
        assert "    request_timeout: float | None = None," in text, name
        assert "        timeout=request_timeout," in text, name


def test_typescript_operations_forward_the_call_timeout() -> None:
    files = render_typescript(read_operations(SPEC))
    operations = {name: text for name, text in files.items() if name != "index.ts"}
    assert operations
    for name, text in operations.items():
        assert "signal: options.signal, timeoutMs: options.timeoutMs," in text, name

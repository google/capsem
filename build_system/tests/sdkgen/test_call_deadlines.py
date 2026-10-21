"""Every generated operation forwards a per-call HTTP deadline.

Exec and run answer only when the command finishes, which the service
bounds by the request's `timeout_secs` (default one hour). A client whose
only deadline is the transport default gave up on those calls after 30 s
while the command kept running, and retrying agents started it again. The
facades stretch the deadline per call, so the generated layer must carry it.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.sdkgen.operations import Operation, read_operations
from capsem_builder.sdkgen.python_operations import render_operations as render_python
from capsem_builder.sdkgen.rust_operations import render_operations as render_rust
from capsem_builder.sdkgen.typescript_operations import render_operations as render_typescript
from pydantic import ValidationError

SPEC = Path(__file__).resolve().parents[3] / "sdk/specification/openapi.json"


def test_python_operations_accept_and_forward_a_request_timeout() -> None:
    files = render_python(read_operations(SPEC))
    operations = {name: text for name, text in files.items() if name != "__init__.py"}
    assert operations
    for name, text in operations.items():
        assert "    request_timeout: float | None = None," in text, name
        if name != "pull_image.py":
            assert "        timeout=request_timeout," in text, name


def test_typescript_operations_forward_the_call_timeout() -> None:
    files = render_typescript(read_operations(SPEC))
    operations = {name: text for name, text in files.items() if name != "index.ts"}
    assert operations
    for name, text in operations.items():
        if name != "pullImage.ts":
            assert "signal: options.signal, timeoutMs: options.timeoutMs," in text, name


def test_image_pull_deadline_is_carried_to_all_generated_clients() -> None:
    routes = read_operations(SPEC)
    pull = next(route.operation for route in routes if route.operation.operation_id == "pullImage")
    assert pull.request_timeout_ms == 1_020_000
    python = render_python(routes)["pull_image.py"]
    assert "request_timeout if request_timeout is not None else max(transport.timeout, 1020.0)" in python
    typescript = render_typescript(routes)["pullImage.ts"]
    assert "options.timeoutMs ?? Math.max(transport.timeoutMs, 1020000)" in typescript
    rust = render_rust(routes)["pull_image.rs"]
    assert ".unwrap_or_else(|| transport.timeout().max(Duration::from_millis(1020000)))" in rust


@pytest.mark.parametrize("deadline", [0, -1, True, "1020000", 1.5])
def test_invalid_operation_deadlines_are_refused(deadline: object) -> None:
    operation = next(route.operation for route in read_operations(SPEC) if route.operation.operation_id == "pullImage")
    data = operation.model_dump(by_alias=True, exclude_unset=True)
    data["x-capsem-request-timeout-ms"] = deadline
    with pytest.raises(ValidationError):
        Operation.model_validate(data)

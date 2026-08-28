"""The build and release gate, as callable code instead of recipe bodies.

The justfile used to carry roughly nineteen hundred lines of inline `bash`
across thirty-five recipes. None of it could be unit tested, so every defect in
it was found by running the forty-minute gate and reading the wreckage: a
manifest URL handed to an installer before anything wrote the manifest, a
version built from `$(date +%s)`, a log stream read by a name that rotation had
already moved. Each was a decision no test could reach.

The rule now is that the justfile dispatches and this package decides.
`tests/test_gate_boundary.py` holds both halves: the justfile to a body it
cannot grow logic into, and this package to modules small enough to keep
reading.
"""

from __future__ import annotations

from pathlib import Path

from .errors import GateError


def project_root() -> Path:
    """The checkout this gate belongs to.

    The gate only ever runs from a source tree -- it drives `cargo`, `docker`,
    and the justfile itself -- so an installed copy with no checkout around it
    is a wiring mistake worth naming rather than a case to fall back from.
    """
    root = Path(__file__).resolve().parents[3]
    if not (root / "justfile").is_file():
        raise GateError(f"capsem_builder.gate must run from a checkout; {root} has no justfile")
    return root


__all__ = ["GateError", "project_root"]

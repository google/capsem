"""The one failure the gate raises on purpose.

Everything under `capsem.gate` reports an operator-fixable problem by raising
`GateError`. `cli.main` turns it into a one-line message on stderr and a
non-zero exit -- so a module never calls `sys.exit`, and a traceback in the
gate output always means a defect in this package rather than a bad checkout.
"""

from __future__ import annotations


class GateError(Exception):
    """A condition the operator can fix, reported without a traceback."""

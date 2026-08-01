"""Storage a later step still needs may not be reclaimed by an earlier one.

`just test` found this the only way it could be found. `install-image` ended by
releasing the linux-rust builder rail, and 164 milliseconds later
`cache-ownership` asked Docker to run that exact image and got exit 125 --
"no such image". The run log's own timeline is what showed it:

    16.7  exec docker-storage-policy.py release --boundary after-linux-rust-builder
    16.7  step.end install-image ok
    16.9  exec docker run ... capsem-host-builder:latest ... -> 125

The release was a statement inside another step's body. As a statement it has
no edges, so nothing can order it and no plan check can see it -- while the
identical release also existed as a properly ordered step, hanging off the lane
that finishes with the image. In the shell the preflight ran after that lane
and the ordering held by accident of line order. Composed into one plan the
preflight deliberately runs first, and the accident stopped holding.

That duplication is the signature, and it is what this file watches for: a rail
handed back from two places is a rail whose second owner cannot be ordered.
"""

from __future__ import annotations

import ast
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
GATE = PROJECT_ROOT / "src" / "capsem" / "gate"

#: The reclaim is spelled here, and a step is the only thing that may spell it.
OWNER = "storage.py"


def _released_as_steps() -> set[str]:
    """Phases the plan hands back through a step, read off the plan itself."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    return {
        label.rsplit(".storage.", 1)[1]
        for label in gate_labels("candidate")
        if ".storage." in label
    }


def _released_inline() -> dict[str, list[str]]:
    """Phases handed back by a statement inside some other step's body."""
    found: dict[str, list[str]] = {}
    for path in sorted(GATE.glob("*.py")):
        if path.name == OWNER:
            continue
        for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
            if (
                isinstance(node, ast.Call)
                and isinstance(node.func, ast.Attribute)
                and node.func.attr == "release"
                and node.args
                and isinstance(node.args[0], ast.Constant)
            ):
                found.setdefault(node.args[0].value, []).append(
                    f"{path.name}:{node.lineno}"
                )
    return found


def test_no_rail_is_handed_back_from_two_places() -> None:
    """One owner per rail, or the second one cannot be ordered against anything.

    `linux-rust-builder` had two: a step hanging off the parity lane, and a
    statement at the end of the install preflight. Composed into one plan the
    preflight runs first, so the statement won -- and deleted an image two
    later steps were about to run.
    """
    steps = _released_as_steps()
    inline = _released_inline()

    both = {
        phase: sites for phase, sites in inline.items() if phase in steps
    }

    assert not both, (
        "these rails are released both as a plan step and as a statement "
        "inside another step's body, and only the step can be ordered: "
        + "; ".join(f"{phase} at {', '.join(sites)}" for phase, sites in sorted(both.items()))
    )


def test_the_builder_rail_is_released_after_the_lane_that_uses_it() -> None:
    """And the surviving step is on the right side of its consumers."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    labels = list(gate_labels("candidate"))
    if "static.storage.linux-rust-builder" not in labels:
        return  # a Linux host runs the parity lane natively; no image to hold

    release = labels.index("static.storage.linux-rust-builder")
    for consumer in ("cache-ownership", "linux-rust"):
        assert labels.index(consumer) < release, (
            f"{consumer} runs the builder image after the step that releases it"
        )

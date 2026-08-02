"""Every module command owns what it needs to start from a clean machine.

The release contract requires each module to be runnable on its own: "Each
module must own its prerequisites and must also be executable independently in
a clean local environment. Never rely on a package installed incidentally by an
earlier workflow job or by a developer machine."

`install` did not. Its plan was one step, and `docker/Dockerfile.install-test`
is `FROM capsem-host-builder:latest` -- an image built by a *different* phase.
Inside the complete gate that phase happens to run first; on its own, and on
any machine where the previous run released the tag at `after-install`, the
build failed with `pull access denied`.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _labels(command: str) -> tuple[str, ...]:
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    return gate_labels(command)


def test_install_builds_the_image_its_dockerfile_derives_from() -> None:
    labels = _labels("install")

    assert "host-image" in labels, (
        "Dockerfile.install-test is FROM capsem-host-builder:latest, so the "
        "install lane cannot start on a machine that does not already have it"
    )
    assert labels.index("host-image") < labels.index("install")


def test_composing_install_into_the_gate_does_not_duplicate_the_image() -> None:
    """`plan.shared` makes the second consumer a dependant, not a duplicate."""
    labels = _labels("candidate")

    assert labels.count("host-image") == 1

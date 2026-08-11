"""A retry of one install may not quietly change where the product comes from.

The gate hands the postinst a locally built channel and then runs

    dpkg -i "<deb>" 2>&1 || apt-get install -f -y

`apt-get install -f` re-runs the postinst. The postinst removed the handoff on
`EXIT` -- including a failing exit -- so the retry found no request and
hydrated from the *public* channel instead.

Two consequences, and the second is the dangerous one:

* the reported error named `release.capsem.org` while the real failure was
  local, which is how a resolution bug in a `file://` channel came to look like
  a production outage; and
* had the retry succeeded, the gate would have "proved" an install hydrated
  from production while believing it had qualified the candidate.

The handoff is the writer's to clear -- `ReleaseGraph.clear_handoff` does it
after the install, on every path -- so a failed postinst must leave it alone.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
POSTINSTALL = PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall"


def test_a_failed_postinstall_leaves_the_handoff_for_the_retry() -> None:
    source = POSTINSTALL.read_text(encoding="utf-8")

    assert "trap 'rm -f \"$CAPSEM_INSTALL_MANIFEST_REQUEST\"' EXIT" not in source, (
        "removing the request on EXIT consumes it on failure too, so the very "
        "next `apt-get install -f` hydrates from the public channel"
    )


def test_the_handoff_is_cleared_once_the_install_has_succeeded() -> None:
    """It is still single-use -- just not single-*attempt*."""
    source = POSTINSTALL.read_text(encoding="utf-8")

    assert (
        'rm -f "$CAPSEM_INSTALL_MANIFEST_REQUEST" '
        '"$CAPSEM_INSTALL_MANIFEST_PAYLOAD"' in source
    ), (
        "a request that is never cleared is inherited by the next install"
    )


def test_the_gates_install_never_silently_falls_back_to_another_channel() -> None:
    """`|| apt-get install -f -y` repairs dependencies; it must not re-hydrate.

    Asserted on the proof that issues it, because the shape of that one
    command is the whole hazard.
    """
    proof = (PROJECT_ROOT / "src" / "capsem" / "gate" / "installproof.py").read_text(
        encoding="utf-8"
    )

    assert "verify_manifest_source" in proof, (
        "nothing checks that the installed product was hydrated from the "
        "channel the gate handed over"
    )

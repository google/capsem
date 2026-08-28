"""The drift report must name the shape the arrayref compromise took."""

from __future__ import annotations

from capsem_builder.gate.tools.audit import dependency_drift as DRIFT


def _pkg(deps: tuple[str, ...] = (), build: bool = False) -> dict:
    return {"build_script": build, "dependencies": list(deps)}


def test_a_crate_gaining_its_first_dependency_is_named_as_such() -> None:
    """The one fact that gave the attack away.

    `arrayref` is about two hundred lines of `macro_rules!` and had no
    dependencies since 2015. Version 0.3.10 declared `proc-macro1`, a typosquat
    of `proc-macro2` whose build script fetched and ran a remote binary. Nothing
    else about the release looked unusual -- the version bump was a patch.
    """
    findings = DRIFT.drift(
        {"arrayref": _pkg(("proc-macro1",))},
        {"arrayref": _pkg()},
    )

    assert len(findings) == 1
    assert "arrayref" in findings[0]
    assert "NO dependencies" in findings[0]
    assert "proc-macro1" in findings[0]


def test_a_new_package_that_builds_is_reported_as_both() -> None:
    """A build script is compile-time code execution; say so when it arrives."""
    findings = DRIFT.drift({"proc-macro1": _pkg(build=True)}, {})

    assert len(findings) == 1
    assert "NEW package proc-macro1" in findings[0]
    assert "build script" in findings[0]


def test_a_crate_gaining_a_build_script_is_reported() -> None:
    """A crate that never compiled anything and suddenly does."""
    findings = DRIFT.drift(
        {"quiet": _pkg(("serde",), build=True)},
        {"quiet": _pkg(("serde",))},
    )

    assert len(findings) == 1
    assert "GAINED A BUILD SCRIPT" in findings[0]


def test_an_ordinary_addition_is_reported_without_the_alarm() -> None:
    """Most changes are ordinary, and reading past them must stay cheap.

    A crate that already had dependencies gaining another is routine; the
    first one is not. Saying both the same way is how a report stops being read.
    """
    findings = DRIFT.drift(
        {"tokio": _pkg(("mio", "bytes"))},
        {"tokio": _pkg(("mio",))},
    )

    assert findings == ["tokio declares new dependencies: bytes"]


def test_an_unchanged_graph_says_nothing() -> None:
    graph = {"serde": _pkg(("serde_derive",)), "arrayref": _pkg()}
    assert DRIFT.drift(graph, graph) == []

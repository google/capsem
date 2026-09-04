"""Every module command owns what it needs to start from a clean machine.

The release contract requires each module to be runnable on its own: "Each
module must own its prerequisites and must also be executable independently in
a clean local environment. Never rely on a package installed incidentally by an
earlier workflow job or by a developer machine."

`install` did not. Its plan was one step, while the input-keyed install helper
derives from the exact local host builder -- an image built by a *different*
phase. Inside the complete gate that phase happened to run first; on its own,
and on any machine where the retained tag was absent, the build failed with
`pull access denied`.

It also validated the canonical asset-and-configuration pair without producing
the configuration half. A wider gate incidentally materialized it, so focused
install testing failed whenever that leftover was absent.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def _labels(command: str) -> tuple[str, ...]:
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    return gate_labels(command)


def test_install_builds_the_image_its_dockerfile_derives_from() -> None:
    labels = _labels("install")

    assert "host-image" in labels, (
        "the install helper derives from the local host builder, so the install "
        "lane cannot start on a machine that does not already have it"
    )
    assert labels.index("host-image") < labels.index("install")


def test_standalone_install_materializes_the_content_it_validates() -> None:
    labels = _labels("install")

    assert "materialize-config" in labels, (
        "the standalone install lane validates the canonical profile-content pair, "
        "so it must materialize that pair instead of inheriting another run's output"
    )
    assert labels.index("materialize-config") < labels.index("install")


def test_manifest_selected_install_does_not_replace_selected_content() -> None:
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import built_command

    command = built_command(
        PROJECT_ROOT,
        "install",
        (("selected_content_root", "selected-content"),),
    )

    assert "materialize-config" not in command._describe().labels


def test_composing_install_into_the_gate_does_not_duplicate_the_image() -> None:
    """`plan.shared` makes the second consumer a dependant, not a duplicate."""
    labels = _labels("candidate")

    assert labels.count("host-image") == 1


def test_install_plan_resolves_the_source_reader_at_execution(monkeypatch) -> None:
    """An earlier test import must not freeze its temporary source reader."""
    from capsem_builder.gate import installplan, sourcecapture

    def replacement(_config):
        return object()

    monkeypatch.setattr(sourcecapture, "require_recorded", replacement)

    assert installplan.sourcecapture.require_recorded is replacement
    assert not hasattr(installplan, "require_recorded")

"""Structured outcomes are produced by pytest hooks; these are source doubles."""

import json
from types import SimpleNamespace

import pytest
from capsem_builder.gate.config import load

from tests.fixtures.oci import native_candidates
from tests.qualification import candidate
from tests.qualification.native_report import NativeReport


def test_actual_hook_outcomes_and_required_group_are_retained(monkeypatch, tmp_path):
    from pathlib import Path

    config = load(Path(__file__).resolve().parents[2]).model_copy(update={"root": tmp_path})
    native = config.functional.native_images
    layout = tmp_path / "layout"
    layout.mkdir()
    monkeypatch.setattr(
        candidate,
        "resolve",
        lambda: SimpleNamespace(
            layout=layout,
            name="capsem-debug",
            digest="sha256:" + "a" * 64,
            capabilities=frozenset(),
        ),
    )
    monkeypatch.setattr(
        native_candidates,
        "source",
        lambda root: native_candidates.Source(commit="b" * 40, tree="c" * 40),
    )
    monkeypatch.setattr(native_candidates, "host_platform", lambda: "linux/amd64")
    path = layout / native.report_name
    plugin = NativeReport(config, path)
    node = "tests/qualification/test_runtime.py::test_shell"
    plugin.pytest_collection_finish(
        SimpleNamespace(items=[SimpleNamespace(nodeid=node, get_closest_marker=lambda name: None)])
    )
    for phase in ("setup", "call", "teardown"):
        plugin.pytest_runtest_logreport(SimpleNamespace(nodeid=node, when=phase, outcome="passed"))
    plugin.pytest_sessionfinish(SimpleNamespace(exitstatus=0), 0)
    value = json.loads(path.read_text())
    assert value["cases"] == {node: {"setup": "passed", "call": "passed", "teardown": "passed"}}
    assert value["groups"] == {"mandatory": 1} and value["exitstatus"] == 0
    assert 0 < value["started_at"] <= value["completed_at"]


def test_report_cannot_be_written_to_unselected_destination(monkeypatch, tmp_path):
    from pathlib import Path

    config = load(Path(__file__).resolve().parents[2])
    monkeypatch.setattr(candidate, "resolve", lambda: SimpleNamespace(layout=tmp_path / "selected"))
    with pytest.raises(ValueError, match="destination"):
        NativeReport(config, tmp_path / "foreign").pytest_sessionfinish(
            SimpleNamespace(exitstatus=0), 0
        )

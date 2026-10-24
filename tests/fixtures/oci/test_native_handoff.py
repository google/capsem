"""Source-only receipt/journal/byte staging proof; no native VM is represented."""

from __future__ import annotations

import json
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.gate.config import load
from capsem_builder.gate.pytestsuite import Suite
from capsem_builder.gate.pythonenv import uv_run

from tests.fixtures.oci import native_candidates as native
from tests.fixtures.oci import native_handoff as handoff
from tests.fixtures.oci import native_proof, producer_evidence
from tests.fixtures.oci.test_pinned_image import _layout
from tests.fixtures.oci.test_producer_evidence import output
from tests.qualification.candidate import declared_capabilities

ROOT = Path(__file__).resolve().parents[3]


def fixture(monkeypatch, tmp_path):
    config = load(ROOT).model_copy(update={"root": tmp_path})
    selected = {}
    source = native.Source(commit="a" * 40, tree="b" * 40)
    monkeypatch.setattr(native, "source", lambda root: source)
    monkeypatch.setattr(native, "host_platform", lambda: "linux/amd64")
    for i, (section, name) in enumerate(
        [("debug_image", "capsem-debug"), ("reference_image", "dev")]
    ):
        layout, digest, _ = _layout(tmp_path / section, architecture="amd64")
        repository = "ghcr.io/google/capsem/" + name
        original = tmp_path / (section + "-original")
        output(original, digest, repository=repository, architecture="amd64")
        producer_evidence.stage(layout, original, repository, "amd64", maximum=1048576)
        image = SimpleNamespace(
            section=section,
            stage=lambda path=layout: path.parent,
            layout_path=lambda value, path=layout: path,
            settings=lambda label=name, repo=repository: SimpleNamespace(
                name=label, repository=repo, evidence_max_bytes=1048576
            ),
        )
        native.record(config, image, digest, "linux/amd64", source)
        receipt, _ = native.selected(config, image)
        caps = declared_capabilities(name)
        groups = {"mandatory": 1, **dict.fromkeys(caps, 1)}
        cases = {
            f"tests/qualification/test_{group}.py::test_{group}": {
                "setup": "passed",
                "call": "passed",
                "teardown": "passed",
            }
            for group in groups
        }
        outcome = {
            "source": source.model_dump(),
            "platform": "linux/amd64",
            "image": name,
            "subject": digest,
            "capabilities": sorted(caps),
            "started_at": 2 + i * 10,
            "completed_at": 3 + i * 10,
            "exitstatus": 0,
            "cases": cases,
            "groups": groups,
        }
        (layout / config.functional.native_images.report_name).write_text(json.dumps(outcome))
        journal = config.path(config.runlog.root) / (f"run-{i}")
        journal.mkdir(parents=True)
        env = {
            config.functional.qualification.image_variable: name,
            config.functional.qualification.layout_variable: str(layout),
            config.functional.native_images.report_variable: str(
                layout / config.functional.native_images.report_name
            ),
        }
        step = "qualification.pytest.qualification"
        rows = [
            {
                "event": "run.start",
                "ts": 1 + i * 10,
                "command": "image-qualify",
                "head": source.commit,
                "platform": "Linux",
                "machine": "x86_64",
                "argv": [
                    "capsem-gate",
                    "image-qualify",
                    "--image",
                    name,
                    "--layout",
                    str(layout),
                    "--native-report",
                ],
            },
            {
                "event": "exec",
                "ts": 4 + i * 10,
                "step": step,
                "exit": 0,
                "env": env,
                "argv": Suite(label="pytest.qualification", paths=("tests/qualification",)).argv(
                    config
                ),
            },
            {"event": "step.end", "ts": 4.5 + i * 10, "step": step, "status": "ok"},
            {"event": "run.end", "ts": 5 + i * 10, "status": "ok"},
        ]
        for row in rows:
            row.update(schema=config.runlog.event_schema, run_id=journal.name)
        (journal / config.runlog.events).write_text("".join(json.dumps(row) + "\n" for row in rows))
        producer = config.path(config.runlog.root) / (f"build-{i}")
        producer.mkdir()
        command = "debug-image" if section == "debug_image" else "reference-image"
        settings = getattr(config.functional, section)
        produced_rows = [
            {
                "event": "run.start",
                "command": command,
                "head": source.commit,
                "platform": "Linux",
                "machine": "x86_64",
                "argv": ["capsem-gate", command, "--candidate-receipt"],
            },
            {
                "event": "exec",
                "step": command + ".build",
                "argv": uv_run(
                    config,
                    "python",
                    config.root / settings.script,
                    section,
                    "build",
                    "--platform",
                    "linux/amd64",
                    "--receipt",
                ),
                "exit": 0,
            },
            {"event": "step.end", "step": command + ".build", "status": "ok"},
            {"event": "run.end", "status": "ok"},
        ]
        for row in produced_rows:
            row.update(schema=config.runlog.event_schema, run_id=producer.name)
        (producer / config.runlog.events).write_text(
            "".join(json.dumps(row) + "\n" for row in produced_rows)
        )
        selected[section] = (image, receipt, layout, journal)

    @contextmanager
    def leased(config, image):
        # Lease ownership has separate real-flock tests; only this seam is a double.
        yield native.selected(config, image)

    monkeypatch.setattr(handoff, "leased", leased)
    monkeypatch.setattr(handoff, "SECTIONS", {key: row[0] for key, row in selected.items()})
    return config, selected


def test_both_complete_native_records_stage_exact_original_bytes(monkeypatch, tmp_path):
    config, selected = fixture(monkeypatch, tmp_path)
    destination = handoff.stage(config)
    assert {path.name for path in destination.iterdir()} == {
        "debug_image",
        "reference_image",
        "handoff.json",
    }
    for section, (_, receipt, layout, _) in selected.items():
        actual = destination / section / "layout"
        members = handoff.files(
            layout,
            config.functional.native_images.report_name,
            maximum=config.functional.native_images.artifact_max_files,
            metadata_max=config.functional.native_images.receipt_max_bytes,
        )
        assert {p.relative_to(actual) for p in actual.rglob("*") if p.is_file()} == set(members)
        for path in members:
            assert (actual / path).read_bytes() == (layout / path).read_bytes()
        assert (
            json.loads((destination / section / "build.json").read_text())["subject"]
            == receipt.subject
        )


@pytest.mark.parametrize(
    "defect",
    [
        "torn",
        "failed",
        "missing-end",
        "source",
        "platform",
        "subject",
        "process-failed",
        "filtered",
        "wrong-report",
        "stale-report",
        "skipped-step",
    ],
)
def test_bad_native_journal_cannot_publish_artifact(monkeypatch, tmp_path, defect):
    config, selected = fixture(monkeypatch, tmp_path)
    _, _, _, journal = selected["debug_image"]
    path = journal / config.runlog.events
    rows = [json.loads(row) for row in path.read_text().splitlines()]
    if defect == "torn":
        path.write_text(path.read_text() + "{")
    else:
        if defect == "failed":
            rows[-1]["status"] = "failed"
        if defect == "missing-end":
            rows.pop()
        if defect == "source":
            rows[0]["head"] = "c" * 40
        if defect == "platform":
            rows[0]["machine"] = "aarch64"
        if defect == "subject":
            rows[0]["argv"][rows[0]["argv"].index("--layout") + 1] = "/foreign"
        if defect == "process-failed":
            rows[1]["exit"] = 1
        if defect == "filtered":
            rows[1]["argv"] += ["-k", "only_one"]
        if defect == "wrong-report":
            rows[1]["env"][config.functional.native_images.report_variable] = "/foreign"
        if defect == "stale-report":
            rows[0]["ts"] = 9
        if defect == "skipped-step":
            rows[-1]["skipped"] = ["qualification.pytest.qualification"]
        path.write_text("".join(json.dumps(row) + "\n" for row in rows))
    with pytest.raises(ValueError):
        handoff.stage(config)
    assert not config.path(config.functional.native_images.artifact_dir).exists()


@pytest.mark.parametrize(
    "defect", ["empty", "skip", "missing-phase", "capability", "group", "failed"]
)
def test_missing_required_test_outcomes_are_not_native_proof(monkeypatch, tmp_path, defect):
    config, selected = fixture(monkeypatch, tmp_path)
    _, receipt, layout, _ = selected["reference_image"]
    path = layout / config.functional.native_images.report_name
    data = json.loads(path.read_text())
    first = next(iter(data["cases"]))
    if defect == "empty":
        data["cases"] = {}
    if defect == "skip":
        data["cases"][first]["call"] = "skipped"
    if defect == "missing-phase":
        del data["cases"][first]["teardown"]
    if defect == "capability":
        data["capabilities"] = []
    if defect == "group":
        data["groups"].pop("mandatory")
    if defect == "failed":
        data["exitstatus"] = 1
    path.write_text(json.dumps(data))
    with pytest.raises(ValueError):
        native_proof.report(config, receipt, layout)


@pytest.mark.parametrize("defect", ["extra", "symlink", "fifo", "bytes", "files", "output-link"])
def test_artifact_membership_and_bounds_are_strict(monkeypatch, tmp_path, defect):
    import os

    config, selected = fixture(monkeypatch, tmp_path)
    layout = selected["debug_image"][2]
    if defect == "extra":
        (layout / "token.env").write_text("must never upload")
    if defect == "symlink":
        (layout / "escape").symlink_to(tmp_path)
    if defect == "fifo":
        os.mkfifo(layout / "special")
    if defect in ("bytes", "files"):
        setting = "artifact_max_bytes" if defect == "bytes" else "artifact_max_files"
        native_config = config.functional.native_images.model_copy(update={setting: 1})
        config = config.model_copy(
            update={
                "functional": config.functional.model_copy(update={"native_images": native_config})
            }
        )
    if defect == "output-link":
        path = config.path(config.functional.native_images.artifact_dir)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.symlink_to(tmp_path / "foreign")
    with pytest.raises(ValueError):
        handoff.stage(config)

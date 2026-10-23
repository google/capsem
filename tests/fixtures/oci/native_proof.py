"""Bounded completed native journals and actual testcase outcomes, never stdout."""

from __future__ import annotations

import json

from capsem_builder.gate import runhistory
from capsem_builder.gate.pytestsuite import Suite
from capsem_builder.gate.pythonenv import uv_run

from . import native_candidates as candidates
from . import producer_evidence


def report(config, receipt, layout):
    settings = config.functional.native_images
    path = candidates.contained(layout, layout / settings.report_name)
    value = json.loads(producer_evidence.read_file(path, settings.receipt_max_bytes))
    expected = {
        "source",
        "platform",
        "image",
        "subject",
        "capabilities",
        "started_at",
        "completed_at",
        "exitstatus",
        "cases",
        "groups",
    }
    if set(value) != expected or (
        value["source"],
        value["platform"],
        value["image"],
        value["subject"],
    ) != (receipt.source.model_dump(), settings.platform, receipt.image, receipt.subject):
        raise ValueError("native test report source/platform/subject differs")
    cases = value["cases"]
    if (
        value["exitstatus"] != 0
        or not isinstance(cases, dict)
        or not cases
        or any(
            not key.startswith(config.functional.qualification.suite_path + "/")
            or phases != {"setup": "passed", "call": "passed", "teardown": "passed"}
            for key, phases in cases.items()
        )
    ):
        raise ValueError("native qualification has missing/skipped/failed test outcomes")
    from tests.qualification.candidate import declared_capabilities

    # Capability declarations are source-owned, not a runner-supplied skip list.
    expected_caps = declared_capabilities(receipt.image)
    if value["capabilities"] != sorted(expected_caps):
        raise ValueError("native qualification capabilities differ from source")
    groups = value["groups"]
    if (
        not isinstance(groups, dict)
        or set(groups) != {"mandatory", *expected_caps}
        or any(type(count) is not int or count <= 0 for count in groups.values())
        or sum(groups.values()) != len(cases)
    ):
        raise ValueError("native qualification missing required capability/mandatory group")
    if not (
        isinstance(value["started_at"], (float, int))
        and isinstance(value["completed_at"], (float, int))
        and 0 < value["started_at"] <= value["completed_at"]
    ):
        raise ValueError("native report time interval invalid")
    return value


def match(config, receipt, layout, outcome, directory):
    settings = config.functional.native_images
    path = candidates.contained(directory.parent, directory / config.runlog.events)
    producer_evidence.read_file(path, settings.journal_max_bytes)
    recovered = runhistory.recover(directory, config.runlog)
    rows = recovered.events
    if recovered.torn or not rows:
        raise ValueError("native qualification journal torn/empty")
    start, end = rows[0], rows[-1]
    if (
        sum(row.get("event") == "run.start" for row in rows) != 1
        or sum(row.get("event") == "run.end" for row in rows) != 1
    ):
        raise ValueError("native journal must contain one complete run")
    if (start.get("event"), end.get("event"), start.get("command")) != (
        "run.start",
        "run.end",
        "image-qualify",
    ):
        raise ValueError("native qualification journal incomplete/wrong command")
    argv = start.get("argv", [])

    def flag(name, expected):
        return argv.count(name) == 1 and argv[argv.index(name) + 1 : argv.index(name) + 2] == [
            expected
        ]

    if (
        not flag("--image", receipt.image)
        or not flag("--layout", str(layout))
        or argv.count("--native-report") != 1
    ):
        raise ValueError("native journal candidate differs")
    if (start.get("head"), start.get("platform"), start.get("machine"), end.get("status")) != (
        receipt.source.commit,
        settings.system,
        settings.machine,
        "ok",
    ):
        raise ValueError("native journal source/host/verdict differs")
    if end.get("failures") or end.get("skipped"):
        raise ValueError("native journal has failures/skipped steps")
    if any(
        row.get("run_id") != start["run_id"] or row.get("schema") != config.runlog.event_schema
        for row in rows
    ):
        raise ValueError("native journal envelope differs")
    if any(
        row.get("event") == "qualification.resume"
        or (row.get("event") in ("step.end", "action") and row.get("status") != "ok")
        for row in rows
    ):
        raise ValueError("native journal contains incomplete/carried/failed work")
    step = "qualification.pytest.qualification"
    executions = [row for row in rows if row.get("event") == "exec" and row.get("step") == step]
    if len(executions) != 1:
        raise ValueError("native journal needs one actual qualification process")
    executed = executions[0]
    suite = Suite(label="pytest.qualification", paths=(config.functional.qualification.suite_path,))
    variables = config.functional.qualification
    env = executed.get("env", {})
    if (
        executed.get("exit") != 0
        or executed.get("argv") != suite.argv(config)
        or env.get(variables.image_variable) != receipt.image
        or env.get(variables.layout_variable) != str(layout)
        or env.get(settings.report_variable) != str(layout / settings.report_name)
    ):
        raise ValueError("native journal actual process/candidate differs")
    if not any(
        row.get("event") == "step.end" and row.get("step") == step and row.get("status") == "ok"
        for row in rows
    ):
        raise ValueError("native journal missing completed qualification step")
    if not (
        start["ts"]
        <= outcome["started_at"]
        <= outcome["completed_at"]
        <= executed["ts"]
        <= end["ts"]
    ):
        raise ValueError("native report is not from this completed journal")
    return path


def completed(config, receipt, layout, outcome):
    settings = config.functional.native_images
    root = config.path(config.runlog.root)
    directories = sorted(path for path in root.iterdir() if path.is_dir() and not path.is_symlink())
    if len(directories) > settings.journal_max_runs:
        raise ValueError("native journal discovery budget exceeded")
    matches = []
    for directory in directories:
        path = directory / config.runlog.events
        if not path.exists():
            continue
        # Only exact image-qualify records are eligible; no best-effort proof.
        data = producer_evidence.read_file(path, settings.journal_max_bytes)
        try:
            first = json.loads(data.splitlines()[0])
        except (ValueError, IndexError) as error:
            raise ValueError("torn candidate journal") from error
        if first.get("command") == "image-qualify" and first.get("head") == receipt.source.commit:
            argv = first.get("argv", [])
            if str(layout) in argv and first["ts"] <= outcome["started_at"]:
                matches.append(match(config, receipt, layout, outcome, directory))
    if len(matches) != 1:
        raise ValueError("native candidate requires exactly one completed matching journal")
    return matches[0]


def built(config, receipt):
    """The existing producer's completed gate, not a mutable tag inspection."""
    settings = config.functional.native_images
    root = config.path(config.runlog.root)
    directories = sorted(path for path in root.iterdir() if path.is_dir() and not path.is_symlink())
    if len(directories) > settings.journal_max_runs:
        raise ValueError("native producer journal budget exceeded")
    command = {"debug_image": "debug-image", "reference_image": "reference-image"}[receipt.section]
    selected = getattr(config.functional, receipt.section)
    expected = uv_run(
        config,
        "python",
        config.root / selected.script,
        receipt.section,
        "build",
        "--platform",
        receipt.platform,
        "--receipt",
    )
    found = []
    for directory in directories:
        path = candidates.contained(root, directory / config.runlog.events)
        if not path.exists():
            continue
        data = producer_evidence.read_file(path, settings.journal_max_bytes)
        try:
            start = json.loads(data.splitlines()[0])
        except (ValueError, IndexError) as error:
            raise ValueError("native producer journal torn") from error
        if start.get("command") != command or start.get("head") != receipt.source.commit:
            continue
        recovered = runhistory.recover(directory, config.runlog)
        rows = recovered.events
        if recovered.torn or (rows[-1].get("event"), rows[-1].get("status")) != ("run.end", "ok"):
            raise ValueError("native producer did not complete")
        if (
            sum(row.get("event") == "run.start" for row in rows) != 1
            or sum(row.get("event") == "run.end" for row in rows) != 1
        ):
            raise ValueError("native producer journal must contain one complete run")
        if (start.get("platform"), start.get("machine")) != (settings.system, settings.machine):
            raise ValueError("native producer host differs")
        if start.get("argv") != ["capsem-gate", command, "--candidate-receipt"]:
            raise ValueError("native producer invocation differs")
        executions = [
            row
            for row in rows
            if row.get("event") == "exec" and row.get("step") == command + ".build"
        ]
        if (
            len(executions) != 1
            or executions[0].get("argv") != expected
            or executions[0].get("exit") != 0
        ):
            raise ValueError("native producer actual process differs")
        if not any(
            row.get("event") == "step.end"
            and row.get("step") == command + ".build"
            and row.get("status") == "ok"
            for row in rows
        ):
            raise ValueError("native producer step did not complete")
        if any(
            row.get("run_id") != start["run_id"]
            or row.get("schema") != config.runlog.event_schema
            or (row.get("event") in ("step.end", "action") and row.get("status") != "ok")
            for row in rows
        ):
            raise ValueError("native producer journal envelope/work differs")
        if rows[-1].get("failures") or rows[-1].get("skipped"):
            raise ValueError("native producer has failed/skipped work")
        found.append(path)
    if len(found) != 1:
        raise ValueError("native candidate requires one completed producer journal")
    return found[0]

"""Preparation/phase source doubles; no Cargo, signer or bench executable runs."""

import json
import os
import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.cache.signedreceipt import FileStamp, PreparedExecutable
from capsem_builder.gate import config
from test_criterion_artifact import protocol as protocol

from benchmarks.helpers import criterion_process as process

ROOT = Path(__file__).resolve().parents[3]


def test_collector_help_loads_owned_helpers_without_ambient_pythonpath(tmp_path):
    environment = dict(os.environ)
    environment.pop("PYTHONPATH", None)
    result = subprocess.run([sys.executable,str(ROOT/"benchmarks/collectors/criterion"),"--help"],
        cwd=tmp_path,env=environment,capture_output=True,text=True,timeout=10)
    assert result.returncode == 0, result.stderr
    assert "--target" in result.stdout


@pytest.fixture
def execution(protocol, monkeypatch):
    metadata, events, root, stage, executable = protocol
    configuration = config.load(ROOT)
    settings = configuration.benchmark.criterion
    (root / ".cargo").mkdir()
    (root / ".cargo/config.toml").write_text((ROOT / ".cargo/config.toml").read_text())
    for relative in (settings.prepare_runner, settings.entitlements):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("source signing policy double")
    calls = []
    state = {"signed": False, "broken": None}
    monkeypatch.setattr(process.config, "load", lambda root: configuration)
    monkeypatch.setattr(
        process, "load_paths", lambda root: SimpleNamespace(stage=lambda name: stage)
    )
    monkeypatch.setattr(process.sys, "platform", "linux")

    def capture(command, **kwargs):
        calls.append((command, kwargs))
        assert kwargs["deadline"] > 0 and kwargs["budget"]["bytes"] > 0
        if command[:2] == ["cargo", "metadata"]:
            return json.dumps(metadata), 1.0
        if command[:2] == ["cargo", "bench"]:
            assert "--no-run" in command and "--message-format=json" in command
            return "\n".join(json.dumps(row) for row in events), 2.0
        if "--prepare-only" in command:
            signed = executable.with_suffix(".signed-copy")
            signed.write_bytes(b"signed source double")
            signed.chmod(0o755)
            receipt = PreparedExecutable(
                schema_version="capsem.signed-executable.v1",
                original=FileStamp.capture(executable),
                executable=FileStamp.capture(signed),
                policy=(
                    FileStamp.capture(root / settings.entitlements),
                    FileStamp.capture(root / settings.prepare_runner),
                ),
                assets_dir=root / settings.assets_dir,
                argv0=str(executable),
            )
            if state["broken"] == "binding":
                receipt = receipt.model_copy(update={"assets_dir": Path("/foreign")})
            if state["broken"] == "replacement":
                executable.write_bytes(b"changed")
            return receipt.model_dump_json(), 3.0
        assert command[0] == str(executable) and command[1] == "--bench"
        expected = executable.with_suffix(".signed-copy") if state["signed"] else executable
        assert kwargs["executable"] == expected
        if state["signed"]:
            assert kwargs["environment"]["CAPSEM_ASSETS_DIR"] == str(root / settings.assets_dir)
        if state["broken"] == "runtime":
            expected.write_bytes(b"changed")
        return (
            "group/case: benchmark\n" if command[-1] == "--list" else "measured diagnostics\n"
        ), 4.0

    monkeypatch.setattr(process, "capture", capture)

    def prepare():
        return process.prepare(
            root,
            "capsem-criterion-protocol-fixture",
            "criterion_protocol",
            timeout=60,
            budget={"bytes": 1_000_000},
            settings=settings,
        )

    return prepare, calls, state, executable, root


@pytest.mark.parametrize("signed", [False, True])
def test_exact_prepared_executable_declares_and_measures_with_explicit_bench(
    execution, monkeypatch, signed
):
    prepare, calls, state, executable, root = execution
    state["signed"] = signed
    if signed:
        monkeypatch.setattr(process.sys, "platform", "darwin")
    ready = prepare()
    assert ready.run(["--list"], root / "list", "declare") == "group/case: benchmark\n"
    assert (
        ready.run(["--sample-size", "10"], root / "measured", "measure") == "measured diagnostics\n"
    )
    runtime = calls[-2:]
    assert runtime[0][0] == [str(executable), "--bench", "--list"]
    assert runtime[1][0] == [str(executable), "--bench", "--sample-size", "10"]
    assert runtime[0][1]["executable"] == runtime[1][1]["executable"]
    assert all("CRITERION_HOME" not in args["environment"] for _, args in calls[:2])
    assert ready.receipt["metadata"]["elapsed_seconds"] == 1
    assert ready.receipt["prepare"]["elapsed_seconds"] == 2
    assert ready.receipt["measure"]["elapsed_seconds"] == 4


@pytest.mark.parametrize("broken", ["binding", "replacement"])
def test_signed_handoff_rejects_foreign_bindings_and_original_races(execution, monkeypatch, broken):
    prepare, calls, state, _, _ = execution
    state["broken"] = broken
    monkeypatch.setattr(process.sys, "platform", "darwin")
    with pytest.raises(ValueError):
        prepare()
    assert len(calls) == 3


@pytest.mark.parametrize("when", ["before", "after", "between"])
def test_artifact_replacement_fails_before_or_after_phase(execution, when):
    prepare, calls, state, executable, root = execution
    ready = prepare()
    if when == "before":
        executable.write_bytes(b"changed")
    elif when == "after":
        state["broken"] = "runtime"
    else:
        ready.run(["--list"], root / "list", "declare")
        executable.write_bytes(b"changed between phases")
    before = len(calls)
    with pytest.raises(ValueError, match="replaced"):
        ready.run([], root / "measured", "measure")
    assert len(calls) == before + (when == "after")


@pytest.mark.parametrize(
    "override", ["CARGO_BUILD_TARGET", "CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER", "config"]
)
def test_unsupported_runner_or_target_fails_before_children(execution, monkeypatch, override):
    prepare, calls, _, _, root = execution
    if override == "config":
        (root / ".cargo/config.toml").write_text('[target.unknown]\nrunner="foreign"')
    else:
        monkeypatch.setenv(override, "foreign")
    with pytest.raises(ValueError):
        prepare()
    assert not calls


@pytest.mark.parametrize("failure", ["budget", "utf8", "exit", "deadline"])
def test_exact_byte_capture_budgets_decode_and_original_status(monkeypatch, tmp_path, failure):
    budget = {"bytes": 3}
    called = []

    def tee(command, **kwargs):
        called.append(command)
        assert kwargs["stderr"] is None and kwargs["timeout_seconds"] > 0
        kwargs["write_bytes"](
            b"1234" if failure == "budget" else b"\xff" if failure == "utf8" else b"ok"
        )
        return 7 if failure == "exit" else 0

    monkeypatch.setattr(process.processgroup, "tee", tee)
    expected = subprocess.CalledProcessError if failure == "exit" else ValueError
    with pytest.raises(expected):
        process.capture(
            ["source-double"],
            root=tmp_path,
            environment=dict(os.environ),
            deadline=0 if failure == "deadline" else process.time.monotonic() + 2,
            budget=budget,
            policy=process.processgroup.StopPolicy(1, 0.01),
        )
    assert bool(called) == (failure != "deadline")

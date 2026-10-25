"""Benchmark setup waits for observed idle load, never for other objections."""

import json
import subprocess

import pytest

from tests.ironbank.kingslanding import test_publish_benchmark as benchmark


def doctor(exit_code, *objections):
    return subprocess.CompletedProcess(
        [],
        exit_code,
        json.dumps({"objections": [{"what": name} for name in objections]}).encode(),
        b"",
    )


def scripted_doctor(monkeypatch, replies):
    calls = []

    def run(argv, **options):
        calls.append((argv, options))
        return replies[min(len(calls) - 1, len(replies) - 1)]

    monkeypatch.setattr(benchmark.subprocess, "run", run)
    return calls


def test_load_must_become_fit_before_measurement(monkeypatch, tmp_path):
    calls = scripted_doctor(monkeypatch, [doctor(1, "load"), doctor(0)])
    elapsed = [0.0]
    monkeypatch.setattr(benchmark, "monotonic", lambda: elapsed[0], raising=False)
    monkeypatch.setattr(
        benchmark, "sleep", lambda delay: elapsed.__setitem__(0, elapsed[0] + delay), raising=False
    )

    benchmark.qualify_benchmark_host(tmp_path)

    assert len(calls) == 2 and elapsed[0] > 0
    assert (tmp_path / "doctor.exit").read_text() == "0\n"
    observations = [
        json.loads(line) for line in (tmp_path / "doctor-readiness.jsonl").read_text().splitlines()
    ]
    assert [row["exit_code"] for row in observations] == [1, 0]
    assert observations[0]["fitness"]["objections"] == [{"what": "load"}]


@pytest.mark.parametrize("objections", [("governor",), ("load", "strays"), ()])
def test_standing_or_unexplained_failure_never_waits(monkeypatch, tmp_path, objections):
    calls = scripted_doctor(monkeypatch, [doctor(1, *objections), doctor(0)])
    with pytest.raises(AssertionError):
        benchmark.qualify_benchmark_host(tmp_path)
    assert len(calls) == 1
    assert (tmp_path / "doctor.exit").read_text() == "1\n"


def test_persistent_load_still_fails_at_the_configured_deadline(monkeypatch, tmp_path):
    calls = scripted_doctor(monkeypatch, [doctor(1, "load")])
    elapsed = [0.0]
    monkeypatch.setattr(benchmark, "monotonic", lambda: elapsed[0])
    monkeypatch.setattr(
        benchmark, "sleep", lambda delay: elapsed.__setitem__(0, elapsed[0] + delay)
    )
    with pytest.raises(AssertionError, match="load"):
        benchmark.qualify_benchmark_host(tmp_path)
    assert (
        elapsed[0] == benchmark.for_root(benchmark.PROJECT_ROOT).benchmark.run.fitness_timeout_secs
    )
    assert len(calls) > 1
    assert (tmp_path / "doctor.exit").read_text() == "1\n"


def test_invalid_doctor_output_is_never_retried(monkeypatch, tmp_path):
    calls = scripted_doctor(monkeypatch, [subprocess.CompletedProcess([], 1, b"broken", b"")])
    with pytest.raises(json.JSONDecodeError):
        benchmark.qualify_benchmark_host(tmp_path)
    assert len(calls) == 1

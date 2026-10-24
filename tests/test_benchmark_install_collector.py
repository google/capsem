"""Source contracts only: genuine disposable owners remain native prerequisites."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from benchmark_install_fixture import load, prepared
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[1]


def test_install_dimension_has_a_configured_existing_owner_collector():
    settings = gate_config.load(ROOT).benchmark.install
    assert (
        settings.owner_timeout_seconds + settings.cleanup_timeout_seconds
        < gate_config.load(ROOT).benchmark.run.timeout_secs
    )
    assert (ROOT / "benchmarks/collectors/install").is_file()
    assert settings.inputs.startswith("cache/target/")


def test_missing_genuine_inputs_refuses_before_owner_invocation(tmp_path):
    from benchmarks.helpers.install_inputs import load_inputs

    with pytest.raises(FileNotFoundError):
        load_inputs(
            tmp_path / "missing.json",
            maximum=1024,
            package_maximum=1024,
            source_commit="a" * 40,
            platform="macos",
            architecture="arm64",
        )


@pytest.mark.parametrize("platform", ["linux", "macos"])
def test_admission_verifies_real_package_sbom_and_runtime_bytes(tmp_path, monkeypatch, platform):
    path, declaration = prepared(tmp_path, monkeypatch, platform=platform)
    result = load(path, declaration)
    assert result.pair["before"]["source_commit"] == "a" * 40
    assert result.pair["after"]["source_commit"] == "b" * 40
    assert result.pair["before"]["runtime_sha256"] == result.pair["after"]["runtime_sha256"]
    root = Path(declaration["before"]["runtime_inputs"])
    next((root / "runtime").rglob("rootfs")).write_bytes(b"corrupt")
    with pytest.raises(ValueError, match="mismatch"):
        load(path, declaration)


def test_runtime_minimum_cannot_be_lowered_to_manufacture_a_before_pair(tmp_path, monkeypatch):
    path, declaration = prepared(tmp_path, monkeypatch)
    for phase in ("before", "after"):
        manifest = Path(declaration[phase]["manifest"])
        document = json.loads(manifest.read_text())
        document["runtime"]["min_capsem_version"] = "0.7.1"
        manifest.write_text(json.dumps(document))
        (Path(declaration[phase]["runtime_inputs"]) / "manifest.json").write_bytes(
            manifest.read_bytes()
        )
    with pytest.raises(ValueError, match="minimum"):
        load(path, declaration)


@pytest.mark.parametrize("outcome", ["transport_failure", "http_failure", "succeeded"])
def test_original_request_outcome_is_distinct_from_verified_activation(outcome):
    from benchmarks.helpers.install_metrics import metrics

    request = {"outcome": outcome, "elapsed_ns": 123000000}
    report = {
        "activation": "verified",
        "original_request": request,
        "activation_verified_elapsed_ns": 456000000,
    }
    measured = metrics(report, host_wall_ns=1000000000)
    assert measured["activation.verified_ms"]["samples"] == [456.0]
    assert measured["original_request.time_to_terminal_ms"]["samples"] == [123.0]
    assert measured["original_request.succeeded_count"]["samples"] == [int(outcome == "succeeded")]
    assert "latency_ms" not in json.dumps(measured)


@pytest.mark.parametrize(
    "damage", ["missing_activation", "negative", "clock_order", "unknown_outcome"]
)
def test_unverified_or_impossible_receipts_cannot_become_samples(damage):
    from benchmarks.helpers.install_metrics import metrics

    report = {
        "activation": "verified",
        "original_request": {"outcome": "transport_failure", "elapsed_ns": 100},
        "activation_verified_elapsed_ns": 200,
    }
    if damage == "missing_activation":
        report["activation"] = "failed"
    elif damage == "negative":
        report["original_request"]["elapsed_ns"] = -1
    elif damage == "clock_order":
        report["activation_verified_elapsed_ns"] = 99
    else:
        report["original_request"]["outcome"] = "retry"
    with pytest.raises(ValueError):
        metrics(report, host_wall_ns=1000)

"""Contract-level checks for the canonical content-filter collector (no fake VM proof)."""

from __future__ import annotations

import copy
import json
import runpy
import shlex
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from pydantic import ValidationError

ROOT = Path(__file__).resolve().parents[1]


def collector():
    return runpy.run_path(str(ROOT / "benchmarks/collectors/content-filter"))


def document(module, *, size=4096, concurrency=1):
    settings = module["SETTINGS"]
    metrics, scenarios = {}, []
    for name in module["SCENARIOS"]:
        for field in ("first_body_byte_ms", "latency_ms"):
            metrics[f"guest_capsem.{name}.{field}"] = {
                "unit": "milliseconds",
                "samples": [1.0] * settings.requests,
            }
        size_with_framing = size + (
            module["STREAM_FRAMING_BYTES"] if name == "content_stream" else 0
        )
        scenarios.append(
            {
                "name": name,
                "successful": settings.requests,
                "total_requests": settings.requests,
                "failed": 0,
                "errors": {},
                "transport_retries": 0,
                "transfer_bytes": size_with_framing * settings.requests,
            }
        )
    return {
        "metrics": metrics,
        "sidecar": json.dumps(
            {
                "mock_server_protocol": {
                    "lane": "guest_capsem",
                    "query": f"bytes={size}&delay_ms={settings.stream_delay_ms}",
                    "concurrency": concurrency,
                    "selected_scenarios": list(module["SCENARIOS"]),
                    "scenarios": scenarios,
                }
            }
        ),
    }


def test_command_uses_shared_engine_and_explicit_config_owned_controls():
    module = collector()
    args = shlex.split(module["command"](65536, 2))
    assert args[:2] == ["capsem-bench", "protocol"]
    assert (
        args[args.index("--query") + 1]
        == f"bytes=65536&delay_ms={module['SETTINGS'].stream_delay_ms}"
    )
    assert args[args.index("--concurrency") + 1] == "2"
    assert args[args.index("--collector-out") + 1] == module["ARTIFACT"]
    settings = gate_config.load(ROOT).benchmark.content_filter
    assert [profile.name for profile in settings.profiles] == [
        "disabled",
        "pii",
        "secret",
        "utf8",
    ]
    assert set(settings.body_bytes) == {4096, 65536, 1048576}


def test_validator_preserves_complete_raw_samples():
    module = collector()
    raw = document(module)
    original = copy.deepcopy(raw)
    report = module["validate"](raw, 4096, 1)
    assert report["scenarios"][0]["successful"] == module["SETTINGS"].requests
    assert raw == original


@pytest.mark.parametrize(
    "failure", ["failed", "retries", "truncated", "lost_sample", "wrong_cohort"]
)
def test_validator_refuses_poisoned_or_incomplete_runs(failure):
    module = collector()
    raw = document(module)
    sidecar = json.loads(raw["sidecar"])
    report = sidecar["mock_server_protocol"]
    if failure == "failed":
        report["scenarios"][0]["failed"] = 1
    elif failure == "retries":
        report["scenarios"][0]["transport_retries"] = 1
    elif failure == "truncated":
        report["scenarios"][0]["transfer_bytes"] -= 1
    elif failure == "wrong_cohort":
        report["concurrency"] = 2
    else:
        raw["metrics"]["guest_capsem.content_text.first_body_byte_ms"]["samples"].pop()
    raw["sidecar"] = json.dumps(sidecar)
    with pytest.raises(AssertionError):
        module["validate"](raw, 4096, 1)


def test_config_refuses_work_that_exceeds_inspection_admission():
    settings = gate_config.load(ROOT).benchmark.content_filter
    for values in (
        {"concurrency": [3]},
        {"body_bytes": [16777217]},
        {"stream_delay_ms": 1001},
        {"profiles": []},
    ):
        with pytest.raises(ValidationError):
            type(settings).model_validate({**settings.model_dump(), **values})

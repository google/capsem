#!/usr/bin/env python3
"""Archive Criterion benchmark output as committed benchmark artifacts."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parent.parent
TESTS_DIR = PROJECT_ROOT / "tests"
if str(TESTS_DIR) not in sys.path:
    sys.path.insert(0, str(TESTS_DIR))

from helpers.benchmark_artifacts import (  # noqa: E402
    benchmark_arch,
    benchmark_output_path,
    enrich_benchmark_artifact,
)

SECURITY_ENGINE_SCHEMA = "capsem.security-engine-benchmark.v1"

SUITES = {
    "cel_microbench": {
        "kind": "criterion_cel_microbench",
        "command": "cargo bench -p capsem-security-engine --bench security_engine_cel",
        "prefixes": (
            "security_engine_cel_compile/",
            "security_engine_cel_evaluate/",
            "security_engine_detection_evaluate/",
            "security_engine_backtest_dedupe/",
            "security_engine_runtime_backtest_hunt/",
            "security_engine_model_response_runtime/",
            "security_engine_runtime_registry/",
            "security_engine_policy_context/",
            "security_engine_native_lookup/",
        ),
        "notes": [
            "Host-side microbenchmark only.",
            "Measures canonical policy-context CEL paths, detection evaluation, backtest dedupe, runtime registry operations, compiled-plan rebuild cost, and native lookup comparators.",
            "Does not include guest transport, service IPC, Security Engine emitter, or session.db journal write latency.",
        ],
    },
    "detection_ir_microbench": {
        "kind": "criterion_detection_ir_microbench",
        "command": "cargo bench -p capsem-security-engine --bench detection_ir",
        "prefixes": (
            "security_engine_detection_ir_parse/",
            "security_engine_detection_ir_lowering/",
            "security_engine_detection_ir_matching/",
        ),
        "notes": [
            "Host-side microbenchmark only.",
            "Measures Detection IR V1 JSON parse/validate, Detection IR to CEL detection-rule lowering, lower-plus-compile costs, direct matching, canonical SecurityEvent matching, and lowered-CEL matching.",
            "Does not include VM transport, service IPC, runtime registry propagation, Security Engine dispatch, or session.db journal write latency.",
        ],
    },
    "provider_model_parser_microbench": {
        "kind": "criterion_provider_model_parser_microbench",
        "category": "network-engine",
        "command": "cargo bench -p capsem-core --bench provider_model_parser",
        "prefixes": (
            "provider_model_parser_openai/",
        ),
        "notes": [
            "Host-side microbenchmark only.",
            "Measures OpenAI provider response parsing from SSE frames into canonical model summaries, including single-frame text, multi-frame text, malformed unknown-only responses, provider tool calls, and gzip decode plus parse.",
            "Does not include socket I/O, TLS, HTTP framing, Security Engine evaluation, telemetry emission, or session.db journal write latency.",
        ],
    },
    "mitm_pipeline_microbench": {
        "kind": "criterion_mitm_pipeline_microbench",
        "category": "network-engine",
        "command": "cargo bench -p capsem-core --bench mitm_pipeline",
        "prefixes": (
            "mitm_security_callback_http/",
            "mitm_security_callback_model/",
        ),
        "notes": [
            "Host-side microbenchmark only.",
            "Measures MITM canonical security-event construction plus SecurityEngine callback evaluation for HTTP request, HTTP response, model request, and model response events.",
            "Does not include socket I/O, TLS, HTTP framing, provider response parsing, telemetry emission, or session.db journal write latency.",
        ],
    },
}


def project_version(project_root: Path) -> str:
    cargo = project_root / "Cargo.toml"
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return match.group(1) if match else "unknown"


def source_commit(project_root: Path) -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=project_root,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except Exception:
        return "unknown"


def criterion_measurements(
    criterion_dir: Path,
    prefixes: tuple[str, ...],
) -> list[dict[str, Any]]:
    measurements = []
    for benchmark_json in sorted(criterion_dir.glob("**/new/benchmark.json")):
        benchmark = json.loads(benchmark_json.read_text())
        full_id = benchmark.get("full_id") or benchmark.get("title")
        if not full_id or not full_id.startswith(prefixes):
            continue

        estimates_path = benchmark_json.with_name("estimates.json")
        if not estimates_path.exists():
            raise FileNotFoundError(f"missing estimates for {full_id}: {estimates_path}")
        estimates = json.loads(estimates_path.read_text())

        group, name = split_full_id(full_id)
        slope = estimates.get("slope")
        mean = estimates["mean"]
        median = estimates["median"]
        primary = slope or mean
        measurement = {
            "group": group,
            "name": name,
            "full_id": full_id,
            "estimate_kind": "slope" if slope else "mean",
            "estimate_ns": primary["point_estimate"],
            "estimate_ci_ns": primary["confidence_interval"],
            "estimate_standard_error_ns": primary["standard_error"],
            "mean_ns": mean["point_estimate"],
            "mean_ci_ns": mean["confidence_interval"],
            "median_ns": median["point_estimate"],
            "median_ci_ns": median["confidence_interval"],
        }
        if slope:
            measurement["slope_ns"] = slope["point_estimate"]
            measurement["slope_ci_ns"] = slope["confidence_interval"]
            measurement["slope_standard_error_ns"] = slope["standard_error"]
        if benchmark.get("throughput") is not None:
            measurement["throughput"] = benchmark["throughput"]
        measurements.append(measurement)
    return measurements


def split_full_id(full_id: str) -> tuple[str, str]:
    if "/" not in full_id:
        return full_id, ""
    group, name = full_id.rsplit("/", 1)
    return group, name


def artifact_path(
    project_root: Path,
    version: str,
    arch: str,
    suffix: str,
    category: str = "security-engine",
) -> Path:
    path = benchmark_output_path(project_root, category, version, arch)
    return path.with_name(path.stem + f"_{suffix}.json")


def archive_suite(
    *,
    project_root: Path,
    criterion_dir: Path,
    suffix: str,
    config: dict[str, Any],
) -> Path:
    version = project_version(project_root)
    arch = benchmark_arch()
    measurements = criterion_measurements(criterion_dir, config["prefixes"])
    if not measurements:
        raise RuntimeError(f"no Criterion measurements found for {suffix} in {criterion_dir}")

    data = {
        "schema": SECURITY_ENGINE_SCHEMA,
        "kind": config["kind"],
        "source_commit": source_commit(project_root),
        "profile": {
            "cargo_profile": "bench",
            "criterion_samples": 100,
            "criterion_warmup_seconds": 3,
            "criterion_target_seconds": 5,
        },
        "scope": {
            "vm_originated": False,
            "notes": config["notes"],
        },
        "measurements": measurements,
    }
    data = enrich_benchmark_artifact(
        data,
        project_root=project_root,
        project_version=version,
        arch=arch,
        command=config["command"],
    )

    out_path = artifact_path(
        project_root,
        version,
        arch,
        suffix,
        config.get("category", "security-engine"),
    )
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(data, indent=2) + "\n")
    return out_path


def main() -> int:
    criterion_dir = PROJECT_ROOT / "target" / "criterion"
    written = []
    for suffix, config in SUITES.items():
        written.append(
            archive_suite(
                project_root=PROJECT_ROOT,
                criterion_dir=criterion_dir,
                suffix=suffix,
                config=config,
            )
        )
    for path in written:
        print(f"Criterion benchmark archived to {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

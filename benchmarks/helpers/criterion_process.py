"""Prepare, declare and run one exact Cargo executable through existing owners.

The gate owns subprocess trees/drain/deadlines; cache policy owns shared Cargo;
the existing macOS runner owns signing. This module adds no statistics/sampler.
Original protocol bytes are bounded before decoding and retained in the sidecar.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

from capsem_builder.cache.config import load_paths
from capsem_builder.cache.signedreceipt import FileStamp, PreparedExecutable
from capsem_builder.gate import config, processgroup
from capsem_builder.gate.benchmarkschema import BenchmarkCriterionConfig

from .criterion_artifact import select_executable, select_target


def capture(
    command: list[str],
    *,
    root: Path,
    environment: dict[str, str],
    deadline: float,
    budget: dict[str, int],
    policy: processgroup.StopPolicy,
    executable: Path | None = None,
    observer: processgroup.ForegroundObserver | None = None,
) -> tuple[str, float]:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise ValueError("Criterion phase deadline exceeded")
    output = bytearray()

    def write(chunk: bytes) -> None:
        if len(chunk) > budget["bytes"]:
            raise ValueError("Criterion protocol byte budget exceeded during drain")
        budget["bytes"] -= len(chunk)
        output.extend(chunk)

    started = time.monotonic()
    status = processgroup.tee(
        command,
        cwd=root,
        env=environment,
        write_bytes=write,
        stderr=None,
        executable=executable,
        policy=policy,
        timeout_seconds=remaining,
        observer=observer,
    )
    elapsed = time.monotonic() - started
    if status:
        raise subprocess.CalledProcessError(status, command)
    return output.decode("utf-8", errors="strict"), elapsed


@dataclass
class Execution:
    root: Path
    artifact: FileStamp
    settings: BenchmarkCriterionConfig
    policy: processgroup.StopPolicy
    deadline: float
    budget: dict[str, int]
    signed: PreparedExecutable | None = None
    receipt: dict = field(default_factory=dict)
    runtime_metrics: dict = field(default_factory=dict)

    def verify(self) -> None:
        self.artifact.verify()
        if self.signed:
            self.signed.verify(
                original=self.artifact.path,
                policy=(
                    self.root / self.settings.entitlements,
                    self.root / self.settings.prepare_runner,
                ),
                assets_dir=self.root / self.settings.assets_dir,
            )

    def run(self, arguments: list[str], home: Path, phase: str) -> str:
        if phase not in ("declare", "measure"):
            raise ValueError("unknown Criterion runtime phase")
        self.verify()
        environment = {**os.environ, "CRITERION_HOME": str(home)}
        if self.signed:
            environment["CAPSEM_ASSETS_DIR"] = str(self.signed.assets_dir)
        executable = self.signed.executable.path if self.signed else self.artifact.path
        command = [str(self.artifact.path), "--bench", *arguments]
        observer = resource_observer(self.settings.resources) if phase == "measure" else None
        text, elapsed = capture(
            command,
            root=self.root,
            environment=environment,
            deadline=self.deadline,
            budget=self.budget,
            policy=self.policy,
            executable=executable,
            observer=observer,
        )
        self.verify()
        self.receipt[phase] = {
            "argv": command,
            "executable": str(executable),
            "elapsed_seconds": elapsed,
        }
        if observer is not None:
            self.runtime_metrics, self.receipt["runtime_resources"] = observer.result()
        return text


def resource_observer(settings):
    from .criterion_resources import CriterionResources

    return CriterionResources(settings)


def prepare(
    root: Path,
    crate: str,
    target: str,
    *,
    timeout: float,
    budget: dict[str, int],
    settings: BenchmarkCriterionConfig,
) -> Execution:
    root = root.resolve()
    configuration = config.load(root)
    policy = processgroup.StopPolicy.from_execution(configuration.execution)
    deadline = time.monotonic() + timeout
    cargo = tomllib.loads((root / ".cargo/config.toml").read_text())
    targets = cargo.get("target", {})
    runners = {key: row["runner"] for key, row in targets.items() if "runner" in row}
    if runners != {'cfg(target_os = "macos")': settings.prepare_runner}:
        raise ValueError("unsupported Cargo runner configuration for direct Criterion execution")
    if cargo.get("build", {}).get("target") or any(
        key == "CARGO_BUILD_TARGET" or (key.startswith("CARGO_TARGET_") and key.endswith("_RUNNER"))
        for key in os.environ
    ):
        raise ValueError("cross-target or overridden Cargo runners are unsupported")
    stage = load_paths(root).stage("cargo").resolve()
    metadata_command = [
        "cargo",
        "metadata",
        "--locked",
        "--offline",
        "--no-deps",
        "--format-version=1",
    ]
    metadata, metadata_seconds = capture(
        metadata_command,
        root=root,
        environment=dict(os.environ),
        deadline=deadline,
        budget=budget,
        policy=policy,
    )
    selected = select_target(json.loads(metadata), root, stage, crate, target)
    command = [
        "cargo",
        "bench",
        "--locked",
        "-p",
        crate,
        "--bench",
        target,
        "--no-run",
        "--message-format=json",
    ]
    original, build_seconds = capture(
        command,
        root=root,
        environment=dict(os.environ),
        deadline=deadline,
        budget=budget,
        policy=policy,
    )
    events = [json.loads(line) for line in original.splitlines()]
    artifact = select_executable(
        events,
        selected,
        stage,
        settings.artifact_profile.model_dump(),
        profile_directory=settings.profile_directory,
    )
    execution = Execution(
        root,
        artifact,
        settings,
        policy,
        deadline,
        budget,
        receipt={
            "metadata": {
                "argv": metadata_command,
                "stdout": metadata,
                "elapsed_seconds": metadata_seconds,
            },
            "prepare": {"argv": command, "stdout": original, "elapsed_seconds": build_seconds},
            "artifact": artifact.model_dump(mode="json"),
        },
    )
    if sys.platform == "darwin":
        runner = root / settings.prepare_runner
        artifact.verify()
        receipt, seconds = capture(
            [
                str(runner),
                "--prepare-only",
                sys.executable,
                str(budget["bytes"]),
                str(artifact.path),
            ],
            root=root,
            environment=dict(os.environ),
            deadline=deadline,
            budget=budget,
            policy=policy,
        )
        execution.signed = PreparedExecutable.model_validate_json(receipt)
        if execution.signed.original != artifact:
            raise ValueError("Cargo original changed during signed-copy preparation")
        execution.verify()
        execution.receipt["signing"] = {"stdout": receipt, "elapsed_seconds": seconds}
    execution.verify()
    return execution

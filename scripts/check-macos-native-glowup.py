#!/usr/bin/env python3
"""Fail closed unless a macOS native package report contains every full probe."""

from __future__ import annotations

import argparse
import json
import tomllib
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import cast

try:
    from release_glowup import (
        GlowupContractError,
        TransitionKind,
        validate_transition_sequence,
    )
except ModuleNotFoundError:
    from scripts.release_glowup import (
        GlowupContractError,
        TransitionKind,
        validate_transition_sequence,
    )


REQUIRED_CAPABILITIES = (
    "native_install",
    "package_receipt",
    "launchd",
    "physical_vz_boot",
    "full_doctor",
    "installed_winterfell",
)


class NativeGlowupError(RuntimeError):
    """The native package report is incomplete or does not match this source."""


def require_mapping(value: object, field: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise NativeGlowupError(f"macOS glow-up report {field} must be an object")
    return cast(Mapping[str, object], value)


def validate_report(report_path: Path, cargo_toml: Path) -> Mapping[str, object]:
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
        cargo = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        raise NativeGlowupError(f"macOS glow-up evidence is unreadable: {error}") from error
    report = require_mapping(report, "root")
    if report.get("schema") != "capsem.release_glowup.v1":
        raise NativeGlowupError("macOS glow-up report has an unsupported schema")
    if report.get("adapter") != "macos-tart-launchd":
        raise NativeGlowupError("macOS glow-up report came from the wrong adapter")

    artifact = require_mapping(report.get("artifact"), "artifact")
    expected_version = cargo["workspace"]["package"]["version"]
    if artifact.get("version") != expected_version:
        raise NativeGlowupError(
            "macOS glow-up package version does not match Cargo.toml"
        )
    package_sha256 = artifact.get("sha256")
    if not isinstance(package_sha256, str) or len(package_sha256) != 64:
        raise NativeGlowupError("macOS glow-up package digest is missing")

    capabilities = require_mapping(report.get("capabilities"), "capabilities")
    missing = [
        capability
        for capability in REQUIRED_CAPABILITIES
        if capabilities.get(capability) is not True
    ]
    if missing:
        raise NativeGlowupError(
            f"macOS glow-up report lacks required capabilities: {missing}"
        )

    adapter_evidence = require_mapping(
        report.get("adapter_evidence"), "adapter_evidence"
    )
    physical = require_mapping(adapter_evidence.get("physical_vz"), "physical_vz")
    if physical.get("package_sha256") != package_sha256:
        raise NativeGlowupError(
            "physical VZ proof did not use the Tart-installed package"
        )
    for field in ("guest_vm_booted", "full_doctor", "installed_winterfell"):
        if physical.get(field) is not True:
            raise NativeGlowupError(f"physical VZ proof did not pass {field}")
    expected_transitions = (
        TransitionKind.FRESH_INSTALL,
        TransitionKind.TAMPER_REJECTION,
    )
    expected_scope = [kind.value for kind in expected_transitions]
    if report.get("transition_scope") != expected_scope:
        raise NativeGlowupError(
            f"macOS glow-up transition scope must be exactly {expected_scope}"
        )
    transitions = report.get("transitions")
    if not isinstance(transitions, list) or not all(
        isinstance(transition, Mapping) for transition in transitions
    ):
        raise NativeGlowupError("macOS glow-up transitions must be an array of objects")
    transition_rows = cast(list[Mapping[str, object]], transitions)
    try:
        validate_transition_sequence(
            transition_rows,
            expected_transitions=expected_transitions,
        )
    except GlowupContractError as error:
        raise NativeGlowupError(f"macOS glow-up transition proof is invalid: {error}") from error
    return report


def main(arguments: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--cargo-toml", required=True, type=Path)
    args = parser.parse_args(arguments)
    validate_report(args.report, args.cargo_toml)
    print(f"macOS native installed glow-up verified: {args.report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Exact Cargo Criterion artifact and prepared-runner configuration."""

from __future__ import annotations

from typing import Annotated

from pydantic import Field, PositiveInt, StrictBool, StrictInt

from .benchmarkresourceschema import CriterionResourceConfig
from .configschema import Strict


class CriterionArtifactProfile(Strict):
    opt_level: str
    debuginfo: StrictInt
    debug_assertions: StrictBool
    overflow_checks: StrictBool
    test: StrictBool


class BenchmarkCriterionConfig(Strict):
    max_cases: PositiveInt
    quick_sample_size: Annotated[int, Field(ge=10)]
    diagnostic_targets: dict[str, str]
    artifact_profile: CriterionArtifactProfile
    profile_directory: Annotated[str, Field(pattern=r"^[A-Za-z0-9_-]+$")]
    prepare_runner: str
    entitlements: str
    assets_dir: str
    resources: CriterionResourceConfig

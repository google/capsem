"""What `config/gate.toml` says about auditing and qualifying the built product.

Split from `buildschema`, which describes building, signing, and packaging the
product's artifacts and surfaces. Keeping build inputs and qualification suites
in one file left `buildschema` at the 300-line module ceiling this package
enforces on its own source.
"""

from __future__ import annotations

from pathlib import PurePosixPath
from typing import Literal

from pydantic import PositiveFloat, PositiveInt, model_validator

from ..cache.tools import CachedToolPolicy
from .configschema import SafeToken, Strict
from .releaseschema import ReleasePairingEnvironment


class ModulesConfig(Strict):
    build_chain_artifact_tests: tuple[str, ...]
    release_suites: tuple[str, ...]
    contract_globs: tuple[str, ...]
    rust_format: tuple[str, ...]
    rust_coverage: tuple[str, ...]
    rust_coverage_floors: tuple[str, ...]
    rust_coverage_report: str
    rust_coverage_ratchet: str
    rust_coverage_workspace_manifest: str
    rust_coverage_crate_minimum: PositiveFloat
    rust_coverage_ratchet_headroom: PositiveFloat
    rust_coverage_crate_floors: dict[str, float]
    rust_coverage_platform_crate_floors: dict[Literal["Darwin", "Linux"], dict[str, float]]
    rust_test_profile_variable: str
    rust_test_profile: str
    rust_doctests: tuple[str, ...]
    guest_binary_tests: tuple[str, ...]
    release_input_dir: str
    release_profile: str
    release_package: str
    verify_inputs_script: str
    prove_profile_assets_script: str
    glowup_script: str
    macos_glowup_script: str
    platform_support_script: str
    macos_glowup_report: str
    macos_report_variable: str
    glowup_work_dir: str
    release_bin_dir: str
    default_bin_dir: str
    #: The local rehearsal of the release lane's pulled path. Every path is
    #: named here rather than derived from `rehearsal_work_dir`, because the
    #: plan and the script both have to agree on all four and a shared root
    #: with two independent join rules is two spellings of one fact.
    rehearsal_script: str
    rehearsal_channel: str
    rehearsal_work_dir: str
    rehearsal_inputs_dir: str
    rehearsal_package: str
    rehearsal_content_root: str
    rehearsal_glowup_work_dir: str
    rehearsal_before_inputs: str
    rehearsal_after_manifest: str
    release_pairing: ReleasePairingEnvironment


class KingslandingConfig(Strict):
    fixture_script: str
    fixture_dir: str
    suite_path: str
    benchmark_paths: tuple[str, ...]


class GreyjoyConfig(Strict):
    suite_path: str


class FunctionalConfig(Strict):
    kingslanding: KingslandingConfig
    greyjoy: GreyjoyConfig
    injection_script: str
    integration_script: str
    binary: str
    assets_dir: str
    config_root: str
    profiles_subdir: str
    node_workspaces: tuple[str, ...]
    sdk_rust_example: tuple[str, ...]
    binary_variable: str
    assets_variable: str
    config_root_variable: str
    assets_dir_variable: str


class DependencyAuditConfig(Strict):
    """One maintained scanner over the configured dependency lockfiles."""

    cache_stage: SafeToken
    lockfiles: tuple[str, ...]
    config: str
    scanner_args: tuple[str, ...]
    timeout_seconds: PositiveInt
    tool: CachedToolPolicy

    @model_validator(mode="after")
    def lockfiles_are_explicit_source_inputs(self) -> DependencyAuditConfig:
        if not self.lockfiles or len(self.lockfiles) != len(set(self.lockfiles)):
            raise ValueError("dependency audit lockfiles must be non-empty and unique")
        for configured in (*self.lockfiles, self.config):
            path = PurePosixPath(configured)
            if path.is_absolute() or ".." in path.parts or path.name == "":
                raise ValueError("dependency audit lockfiles must be repository-relative")
        if self.scanner_args[:2] != ("scan", "source"):
            raise ValueError("dependency audit scanner must use the source scan command")
        return self


class AuditsConfig(Strict):
    cargo: str
    dependencies: str
    dependency_drift: str
    public_surface: str
    source_syntax: str
    hardcoded_selections: str
    surfaces: str
    docker_ignore: tuple[str, ...]
    shell_severity: str
    shell_ignore: tuple[str, ...]
    skills_dir: str
    max_skill_description_chars: PositiveInt
    max_skill_body_lines: PositiveInt
    dependency_policy: DependencyAuditConfig

"""What `config/gate.toml` says about building and releasing the product.

Split from `harnessschema`, which describes the gate running itself. The seam
is whether a section is about the machinery or about what the machinery makes;
one file carrying both was past the module ceiling this package enforces on its
own source.
"""

from __future__ import annotations

from pathlib import PurePosixPath
from typing import Annotated, Literal

from pydantic import PositiveInt, StringConstraints, field_validator, model_validator

from capsem.dockerpolicy import BuildNetwork, ContainerNetwork

from .configschema import Strict


class ModulesConfig(Strict):
    build_chain_artifact_tests: tuple[str, ...]
    release_suites: tuple[str, ...]
    contract_glob: str
    rust_coverage: tuple[str, ...]
    rust_coverage_floor: str
    guest_binary_tests: tuple[str, ...]
    release_input_dir: str
    release_profile: str
    release_package: str
    verify_inputs_script: str
    prove_profile_assets_script: str
    glowup_script: str
    macos_glowup_script: str
    macos_glowup_report: str
    macos_report_variable: str
    glowup_work_dir: str
    channel_switch_work_dir: str
    release_bin_dir: str
    default_bin_dir: str
    channel_switch_cleared: tuple[str, ...]


class FunctionalConfig(Strict):
    injection_script: str
    integration_script: str
    binary: str
    assets_dir: str
    config_root: str
    profiles_subdir: str
    binary_variable: str
    assets_variable: str
    config_root_variable: str
    assets_dir_variable: str


class HostImageConfig(Strict):
    base_dockerfile: str
    lane_dockerfile: str
    base_tag_template: str
    lane_tag: str
    identity_inputs: tuple[str, ...]
    network: Literal[ContainerNetwork.NONE]
    source_build_network: Literal[BuildNetwork.NONE]
    container_output_dir: str
    container_output_contents: str
    extract_to: str
    lane_container: str
    #: The recipe the lane's refusal names when the base image is missing.
    warm_recipe: str
    tag: str
    dockerfile: str
    context: str
    builder_identity_inputs: tuple[str, ...]
    materialize_network: Literal[BuildNetwork.DEFAULT]
    pnpm_version: Annotated[str, StringConstraints(pattern=r"^[0-9]+\.[0-9]+\.[0-9]+$")]
    rust_image: Annotated[str, StringConstraints(pattern=r"^[^\s@]+@sha256:[0-9a-f]{64}$")]
    uv_image: Annotated[str, StringConstraints(pattern=r"^[^\s@]+@sha256:[0-9a-f]{64}$")]
    cargo_tool_args: dict[Annotated[str, StringConstraints(pattern=r"^[A-Z][A-Z0-9_]+$")], str]
    script: str
    mount: str


class SbomConfig(Strict):
    script: str
    output: str
    dist_glob: str
    macos_package: str
    expected_debs: int
    spdx_version: str


class SigningConfig(Strict):
    entitlements: str
    binaries: tuple[str, ...]
    built: tuple[str, ...]
    built_elsewhere: tuple[str, ...]
    guest_crate: str
    release_binary: str

    @model_validator(mode="after")
    def _every_signed_binary_is_built(self) -> SigningConfig:
        """The signed set is a subset of the built set, or signing has no input.

        Two lists that must agree, so the disagreement is a load error naming
        the binary rather than a `codesign: No such file or directory` twenty
        minutes into a run.
        """
        missing = sorted({PurePosixPath(path).name for path in self.binaries} - set(self.built))
        if missing:
            raise ValueError(f"signed but never built: {missing}")
        return self


class FrontendConfig(Strict):
    build_script: str
    build_target: str
    app_crate: str
    profiles: tuple[str, ...]


class LogsConfig(Strict):
    service_log: str
    failure_root: str
    cli: str


class ImageBuildConfig(Strict):
    admin: tuple[str, ...]
    workspace_admin: tuple[str, ...]
    dependency_backend: tuple[str, ...]
    source_config: str
    guest_dir: str
    workspace_root: str
    workspace_guest_dir: str
    lane_templates: tuple[str, ...]
    templates: tuple[str, ...]
    profiles_glob: str
    profile_manifest: str
    config_root: str
    output: str
    doctor_skips: dict[str, str]

    @model_validator(mode="after")
    def _workspace_is_profile_and_arch_scoped(self) -> ImageBuildConfig:
        for field in ("{profile}", "{arch}"):
            if field not in self.workspace_root:
                raise ValueError(f"image workspace_root must contain {field}")
        rendered = self.workspace_root.format(profile="profile", arch="arch")
        path = PurePosixPath(rendered)
        if path.is_absolute() or ".." in path.parts:
            raise ValueError("image workspace_root must remain inside the checkout")
        guest = PurePosixPath(self.workspace_guest_dir)
        if guest.is_absolute() or len(guest.parts) != 1 or guest.name in {"", ".", ".."}:
            raise ValueError("image workspace_guest_dir must be one relative directory")
        return self


class AuditsConfig(Strict):
    cargo: str
    pnpm: str
    python_lock: str
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


class WebSurfacesConfig(Strict):
    script: str
    targets: tuple[str, ...]
    blocks_clippy: str


class PytestConfig(Strict):
    root: str
    citadel: str
    collection_flags: tuple[str, ...]
    base_flags: tuple[str, ...]
    stop_at_first: str
    parallel_flags: tuple[str, ...]
    coverage_flags: tuple[str, ...]
    broad_ignores: tuple[str, ...]
    host_snapshot_serial: tuple[str, ...]
    serial_paths: tuple[str, ...]
    benchmark_baseline: str
    benchmark_deselect: str
    require_artifacts: str
    profile_variable: str
    base_profile: str
    materialized_profiles: str
    test_manifest: str


class SuitesConfig(Strict):
    source_contract: tuple[str, ...]
    pytest: PytestConfig


class ServiceConfig(Strict):
    """The development daemon, on the same rail an installed package uses."""

    binary: str
    process_binary: str
    sync_assets_script: str
    generated_profiles: str
    assets_dir: str
    home_assets: str
    home_profiles: str
    socket: str
    pidfile: str
    retired_config: tuple[str, ...]
    ready_attempts: int
    ready_interval_seconds: float
    log_level: str


class SmokeGroup(Strict):
    name: str
    paths: tuple[str, ...]
    markers: str
    parallel: int = 0


class SmokeConfig(Strict):
    doctor: tuple[str, ...]
    run_id_variable: str
    log: str
    groups: tuple[SmokeGroup, ...]
    serial_groups: tuple[SmokeGroup, ...]


class InitrdConfig(Strict):
    binaries: tuple[str, ...]
    staging: str
    sources: tuple[str, ...]
    freshness_globs: tuple[str, ...]
    freshness_inputs: tuple[str, ...]
    build: tuple[str, ...]
    init: str
    files: tuple[str, ...]
    trees: tuple[str, ...]
    prune: str
    binary_mode: int
    init_mode: int
    manifest: tuple[str, ...]
    hash_assets: str


class ReleaseConfig(Strict):
    line: Annotated[str, StringConstraints(pattern=r"^\d+\.\d+$")]
    source: str
    source_ref_template: str
    tagger_name: Annotated[str, StringConstraints(min_length=1, max_length=128)]
    tagger_email: Annotated[str, StringConstraints(pattern=r"^[^@\s]+@[^@\s]+$")]
    notes: tuple[str, ...]
    fetch_manifest: str
    binaries: str
    profile: tuple[str, ...]
    preflight_dir: str
    channel_source: str
    default_repository: str
    repository_variable: str
    token_variable: str

    @field_validator("source_ref_template")
    @classmethod
    def _source_ref_is_one_commit_derived_tag(cls, template: str) -> str:
        if template != "capsem-source-{source_commit}":
            raise ValueError("release source_ref_template must be capsem-source-{source_commit}")
        return template


class DevLoopConfig(Strict):
    setup_sentinel: str
    dev_lock: str
    tauri: tuple[str, ...]
    frontend_dev: tuple[str, ...]
    frontend_dir: str
    tui: tuple[str, ...]
    surfaces: tuple[str, ...]
    generate_settings: str
    generated_settings_scratch: str
    check_settings: str
    materialize_config: str

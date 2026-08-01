"""What `config/gate.toml` says about building and releasing the product.

Split from `harnessschema`, which describes the gate running itself. The seam
is whether a section is about the machinery or about what the machinery makes;
one file carrying both was past the module ceiling this package enforces on its
own source.
"""

from __future__ import annotations

from .configschema import Strict


class CrateTool(Strict):
    """A cargo-installed tool: how to find it, and how to get it."""

    name: str
    install: tuple[str, ...]


class ToolchainConfig(Strict):
    sync: tuple[str, ...]
    node_workspaces: tuple[str, ...]
    node_install: tuple[str, ...]
    node_env: dict[str, str]
    rust_targets: tuple[str, ...]
    rust_components: tuple[str, ...]
    crates: tuple[CrateTool, ...]


class ModulesConfig(Strict):
    build_chain_artifact_tests: tuple[str, ...]
    release_suites: tuple[str, ...]
    contract_glob: str
    rust_coverage: tuple[str, ...]
    rust_coverage_floor: str
    guest_agent_build: tuple[str, ...]
    guest_binaries: tuple[str, ...]
    guest_binary_root: str
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


class NamedVolume(Strict):
    source: str
    target: str


class HostImageConfig(Strict):
    tag: str
    dockerfile: str
    context: str
    script: str
    output_dir: str
    nextest_dir: str
    mount: str
    container_output: str
    container_home: str
    probe_user: str
    alpine: str
    tmpfs: str
    nextest_mount: str
    cached_volumes: tuple[NamedVolume, ...]
    environment: dict[str, str]


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
    release_binary: str


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
    lane_templates: tuple[str, ...]
    templates: tuple[str, ...]
    profiles_glob: str
    profile_manifest: str
    config_root: str
    output: str
    doctor_skips: dict[str, str]
    required: tuple[str, ...]


class AuditsConfig(Strict):
    cargo: str
    pnpm: str
    python_lock: str
    public_surface: str
    source_syntax: str
    hardcoded_selections: str
    skills_dir: str


class WebSurfacesConfig(Strict):
    script: str
    targets: tuple[str, ...]
    blocks_clippy: str


class PytestConfig(Strict):
    root: str
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
    rebuild_trigger: str


class ReleaseConfig(Strict):
    precheck: tuple[str, ...]
    notes: tuple[str, ...]
    fetch_manifest: str
    publish: str
    binaries: str
    profile: tuple[str, ...]
    preflight_dir: str
    channel_source: str
    default_repository: str
    repository_variable: str
    token_variable: str


class DevLoopConfig(Strict):
    setup_sentinel: str
    dev_lock: str
    tauri: tuple[str, ...]
    frontend_dev: tuple[str, ...]
    frontend_dir: str
    tui: tuple[str, ...]
    surfaces: tuple[str, ...]
    generate_settings: str
    check_settings: str
    materialize_config: str

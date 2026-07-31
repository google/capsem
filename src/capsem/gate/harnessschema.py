"""What `config/gate.toml` says about the gate running itself.

`configschema` describes the product: architectures, packages, assets, the
install proof. This describes the machinery -- what may not run beside what,
who holds the machine, where a run is recorded, how much disk it may occupy,
and the limits the gate holds its own source to.

Split from `configschema` because the two answer different questions and a
single file carrying both was already past the module ceiling that this project
enforces on itself. The `Strict` base is shared, so a typo is still a failure
and not a silent default in either half.
"""

from __future__ import annotations

from pathlib import PurePosixPath

from pydantic import field_validator, model_validator

from .configschema import Strict


class LintConfig(Strict):
    python_roots: tuple[str, ...]
    strict_roots: tuple[str, ...]
    ty_flags: tuple[str, ...]
    ty_ratchet: tuple[str, ...]

    @property
    def relaxed_roots(self) -> tuple[str, ...]:
        return tuple(name for name in self.python_roots if name not in self.strict_roots)


class BoundaryConfig(Strict):
    max_recipe_lines: int
    max_module_lines: int
    remaining_shell_recipes: tuple[str, ...]
    shell_control_flow: tuple[str, ...]
    recipes_with_inline_control_flow: tuple[str, ...]
    direct_machine_access: tuple[str, ...]
    direct_concurrency: tuple[str, ...]
    modules_bypassing_primitives: tuple[str, ...]


class Exclusive(Strict):
    """Something only one step may hold at a time, and why.

    The reason is not decoration. Written as `&` and `wait` in shell, this
    knowledge lived in comments beside the backgrounded job, and an eighth lane
    could violate a constraint recorded three hundred lines away.

    This is the only representation of an exclusive. A dataclass beside it
    would be a second place for one fact to live, which is how the architecture
    mapping ended up spelled four ways. Frozen, so it is hashable and can key
    the lock table the plan builds.
    """

    name: str = ""
    """Filled in at load from the table key that names it."""

    reason: str


class ExecutionConfig(Strict):
    exclusives: dict[str, Exclusive]

    @model_validator(mode="after")
    def _name_exclusives(self) -> ExecutionConfig:
        for key, exclusive in self.exclusives.items():
            if not exclusive.name:
                object.__setattr__(exclusive, "name", key)
        return self


class LockConfig(Strict):
    """One holder at a time, proven by the kernel rather than by a PID file."""

    path: str
    holder_record: str
    report_after_seconds: float
    wait_timeout_seconds: float
    poll_interval_seconds: float


class LocksConfig(Strict):
    gate: LockConfig


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


class WorkspaceConfig(Strict):
    home: str
    run_dir: str
    seeded_dirs: tuple[str, ...]
    benchmark_root: str
    coverage_file: str
    evidence_dir: str


class RunLogConfig(Strict):
    root: str
    events: str
    event_schema: str
    step_log_dir: str
    summary: str
    latest_link: str
    keep_runs: int
    keep_bytes: int
    artifact_digest: str
    slow_action_seconds: float


class DiskConfig(Strict):
    reclaimable: tuple[str, ...]
    required_free_gb: int
    run_footprint_warn_gb: int

    @field_validator("reclaimable")
    @classmethod
    def _stay_inside_the_checkout(cls, paths: tuple[str, ...]) -> tuple[str, ...]:
        """A reclaimer that can be aimed outside the checkout is a delete command.

        Checked at load, with the offending entry named, rather than trusted at
        the call site: the reclaimer removes whole trees, and the difference
        between a relative path and one that escapes upwards is one editing
        mistake.
        """
        for path in paths:
            parts = PurePosixPath(path)
            if parts.is_absolute() or ".." in parts.parts:
                raise ValueError(f"{path!r} must be relative and must not escape upwards")
        return paths

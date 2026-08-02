"""What `config/gate.toml` says about the product the gate builds.

Separated from `config`, which is how it loads, and from `harnessschema`, which
describes the gate running itself. These are plain schemas: the only logic they
carry derives one value from another that is already declared, so nothing here
can disagree with the file.
"""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict


class Strict(BaseModel):
    """Rejects unknown keys, so a typo is a failure and not a silent default."""

    model_config = ConfigDict(extra="forbid", frozen=True)


class Arch(Strict):
    """A build target under every name some tool insists on.

    `name` is the table key in `config/gate.toml`, stamped in at load. There is
    deliberately no second representation of this: a dataclass mirroring these
    fields would be one more place for the mapping to drift.
    """

    name: str = ""
    rust_target: str
    dpkg: str
    gnu: str
    aliases: tuple[str, ...]

    pkg_config_template: str = ""
    """Filled in at load from the `[architectures]` table it belongs to."""

    @property
    def pkg_config_path(self) -> str:
        """Where the cross toolchain's `.pc` files live inside the builder."""
        return self.pkg_config_template.format(gnu=self.gnu)


class StoragePhase(Strict):
    boundary: str
    rail: str


class StorageConfig(Strict):
    policy_script: str
    ensure_space_script: str
    phases: dict[str, StoragePhase]


class PidfileConfig(Strict):
    names: tuple[str, ...]
    term_wait_seconds: float
    kill_wait_seconds: float
    poll_interval_seconds: float
    proc_stat_template: str


class VolumeSpec(Strict):
    source: str
    target: str


class InstallLayout(Strict):
    assets: str
    config: str
    channel: str
    packages: str
    glowup: str
    extra_owned_paths: tuple[str, ...]

    def owned_paths(self, mount: str) -> tuple[str, ...]:
        """Everything the container writes as its own user."""
        return tuple(
            f"{mount}/{path}"
            for path in (
                self.assets, self.config, self.channel, self.packages, self.glowup,
                *self.extra_owned_paths,
            )
        )


class GuestUser(Strict):
    """The unprivileged container user, and the paths it may write to.

    Every path here has to stay off the bind mount: `/src` belongs to the host
    uid, so anything this user writes there fails with EACCES on Linux and only
    on Linux -- which is why four separate release-gate failures had this one
    shape.
    """

    name: str
    home: str
    runtime_dir: str
    tmp: str
    pytest_cache: str
    asset_manifest: str


class InstallSuite(Strict):
    path: str
    glowup_script: str
    macos_report_check: str
    stage_inputs_script: str
    serve_script: str
    sbom_script: str
    web_surface_script: str
    package_name: str


class InstallConfig(Strict):
    container: str
    image: str
    dockerfile: str
    venv: str
    mount: str
    channel: str
    manifest_version: str
    systemd_ready_attempts: int
    systemd_ready_interval_seconds: float
    serve_ready_file: str
    serve_ready_attempts: int
    serve_ready_interval_seconds: float
    vm_devices: tuple[str, ...]
    optional_vm_devices: tuple[str, ...]
    rosetta_binfmt: str
    systemd_command: str
    cgroup_path: str
    tmpfs_paths: tuple[str, ...]
    bin_dir: str
    installed_capsem: str
    capsem_home: str
    manifest_name: str
    sbom_name: str
    candidate_prefix: str
    file_url_scheme: str
    release_site_dir: str
    storage_ledger: str
    test_output_root: str
    install_log_glob: str
    profile_inputs_variable: str
    preinstall_root: str
    admin_relative: str
    request_script: str
    graph_manifest: str
    legacy_projection: str
    layout: InstallLayout
    guest_user: GuestUser
    suite: InstallSuite
    volumes: tuple[VolumeSpec, ...]

    @property
    def preinstall_admin(self) -> str:
        return f"{self.preinstall_root}/{self.admin_relative}"


class PackageProof(Strict):
    container: str
    systemd_ready_attempts: int
    verify_script: str
    shell_proof_script: str
    shell_marker: str
    session_name: str
    shell_timeout_seconds: int
    binaries: tuple[str, ...]
    binaries_without_version: tuple[str, ...]
    status_requires: tuple[str, ...]

    @property
    def versioned_binaries(self) -> tuple[str, ...]:
        return tuple(
            name for name in self.binaries if name not in self.binaries_without_version
        )


class EnvironmentConfig(Strict):
    """The variable names that say which capsem a process is talking to."""

    home: str
    run_dir: str
    benchmark_root: str
    coverage_file: str


class ArtifactsConfig(Strict):
    """The three files a bootable per-architecture asset tree is made of."""

    kernel: str
    initrd: str
    rootfs: str

    @property
    def bootable(self) -> tuple[str, ...]:
        """What must exist for a tree to boot, in build order."""
        return (self.kernel, self.initrd, self.rootfs)


class PackageSigningConfig(Strict):
    """Where the local Tauri signing material lives, and how it is exported."""

    directory: str
    key: str
    password: str
    key_variable: str
    password_variable: str


class PackageConfig(Strict):
    signing: PackageSigningConfig
    manifest_variable: str
    channel_variable: str
    require_proof_variable: str
    builder_image: str
    build_script: str
    proof_selector: str
    release_inputs_name: str
    default_manifest_url: str
    channels: tuple[str, ...]
    default_channel: str
    toolchain_pin: str
    clock_script: str
    cargo_target_mount: str
    package_suffix: str
    dist_dir: str
    target_volume: str
    proof: PackageProof
    volumes: tuple[VolumeSpec, ...]

    def target_volume_for(self, arch: str) -> str:
        return self.target_volume.format(arch=arch)


class AssetsConfig(Strict):
    test_root: str
    profiles_glob: str
    evidence_artifacts: tuple[str, ...]
    failure_tail_lines: int
    shell_proof_timeout_seconds: int
    run_dir_template: str
    admin_command: tuple[str, ...]
    capsem_binary: str
    hash_assets_script: str
    shell_proof_script: str
    container_cleanup_script: str
    cross_platform_probe_image: str
    cross_platform_prefix: str
    cross_platform_probe_command: str
    merged_assets_dir: str
    merged_config_dir: str
    profile_home_dir: str
    failure_evidence_dir: str
    materialized_profiles_dir: str
    current_link: str
    evidence_suffixes: tuple[str, ...]
    evidence_prune_dirs: tuple[str, ...]


class CandidateConfig(Strict):
    keep_awake_command: tuple[str, ...]
    keep_awake_marker: str
    source_digest_script: str
    orphan_script: str
    source_state_file: str
    colima: str
    bootstrap_script: str
    clean_stale_script: str
    generated_settings_script: str
    materialize_script: str
    recipe_suite: tuple[str, ...]
    candidate_budget: tuple[str, ...]
    failure_rail: str
    unknown_head: str


class StampedFile(Strict):
    """A file carrying a copy of the workspace version, and how it spells it."""

    path: str
    kind: str
    key: str


class VersionsConfig(Strict):
    cargo_manifest: str
    tag_prefix: str
    stamped: tuple[StampedFile, ...]


class DoctorConfig(Strict):
    storage_policy: str
    common_script: str

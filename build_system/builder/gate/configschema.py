"""What `config/gate.toml` says about the product the gate builds.

Separated from `config`, which is how it loads, and from `harnessschema`, which
describes the gate running itself. These are plain schemas: the only logic they
carry derives one value from another that is already declared, so nothing here
can disagree with the file.
"""

from __future__ import annotations

from pathlib import PurePosixPath
from typing import Annotated

from pydantic import BaseModel, ConfigDict, StringConstraints, field_validator, model_validator

SafeToken = Annotated[str, StringConstraints(pattern=r"^[A-Za-z0-9][A-Za-z0-9+._-]*$")]


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
    apt_cross_compilers: tuple[SafeToken, ...]
    docker_platform: str
    aliases: tuple[str, ...]

    pkg_config_template: str = ""
    """Filled in at load from the `[architectures]` table it belongs to."""

    @property
    def pkg_config_path(self) -> str:
        """Where the cross toolchain's `.pc` files live inside the builder."""
        return self.pkg_config_template.format(gnu=self.gnu)

    @model_validator(mode="after")
    def cross_compilers_are_nonempty_and_unique(self) -> Arch:
        if not self.apt_cross_compilers or len(self.apt_cross_compilers) != len(
            set(self.apt_cross_compilers)
        ):
            raise ValueError("architecture apt_cross_compilers must be non-empty and unique")
        return self


class OutputRootsConfig(Strict):
    """Canonical repository-generated output owners inside ``cache/target/``."""

    assets: str
    benchmarks: str
    coverage: str
    distribution: str
    gate_runs: str
    materialized_config: str
    packages: str
    test_artifacts: str

    @field_validator("*")
    @classmethod
    def _is_target_owned(cls, value: str) -> str:
        path = PurePosixPath(value)
        if path.is_absolute() or ".." in path.parts or path.parts[:2] != ("cache", "target"):
            raise ValueError(f"generated output root {value!r} must stay under cache/target/")
        return value

    @model_validator(mode="after")
    def _roots_are_distinct(self) -> OutputRootsConfig:
        values = tuple(str(value) for value in self.__dict__.values())
        if len(values) != len(set(values)):
            raise ValueError("generated output roots must be distinct")
        return self


class PidfileConfig(Strict):
    names: tuple[str, ...]
    term_wait_seconds: float
    kill_wait_seconds: float
    poll_interval_seconds: float
    proc_stat_template: str


class InstallEnvironment(Strict):
    """What the install-test image's smoke check is told."""

    test_output_root: str
    project_environment: str
    ci: str


class PackageEnvironment(Strict):
    """What the builder container is told about its target."""

    target_arch: str
    rust_target: str
    dpkg_arch: str
    rust_toolchain: str
    output_dir: str
    build_revision: str


class ReleaseSiteEnvironment(Strict):
    """Where a release-site build reads from and writes to."""

    url: str
    graph: str
    channel_dist: str

    def runtime(self, *, url: object) -> dict[str, str]:
        """The exact package base used while authoring a release graph."""
        return {self.url: str(url)}


class InstallProofEnvironment(Strict):
    """What the in-container proof is told about what it just installed."""

    installed: str
    bin_src: str
    asset_manifest: str
    source_cli: str

    def runtime(
        self,
        *,
        bin_src: object,
        asset_manifest: object,
        source_cli: object,
    ) -> dict[str, str]:
        """The exact installed-package and current-source cohorts under proof."""
        return {
            self.installed: "1",
            self.bin_src: str(bin_src),
            self.asset_manifest: str(asset_manifest),
            self.source_cli: str(source_cli),
        }


class LinuxRustEnvironment(Strict):
    output_dir: str


class EnvironmentConfig(Strict):
    """The variable names that say which capsem a process is talking to.

    And the builders below it. Every one of these is a Capsem protocol name
    rather than a standard process convention -- `HOME` and `TMPDIR` mean what
    they mean everywhere and stay where they are.
    """

    home: str
    run_dir: str
    assets_dir: str
    profiles_dir: str
    benchmark_root: str
    coverage_file: str
    source_checkout: str
    cargo_target: str
    rustc_wrapper: str
    sccache_dir: str
    sccache_cache_size: str
    sccache_base_dir: str
    uv_cache: str
    pnpm_store: str
    source_commit: str
    qualified_source_commit: str
    command_sandbox_mode: str
    install: InstallEnvironment
    package: PackageEnvironment
    release_site: ReleaseSiteEnvironment
    install_proof: InstallProofEnvironment
    linux_rust: LinuxRustEnvironment

    def capsem(self, *, home: object, run_dir: object) -> dict[str, str]:
        """Which capsem a process is talking to."""
        return {self.home: str(home), self.run_dir: str(run_dir)}

    def content(self, *, assets: object = None, profiles: object = None) -> dict[str, str]:
        """Where it finds its assets and profiles. Absent means unchanged."""
        found = {}
        if assets is not None:
            found[self.assets_dir] = str(assets)
        if profiles is not None:
            found[self.profiles_dir] = str(profiles)
        return found


class CandidateConfig(Strict):
    keep_awake_command: tuple[str, ...]
    keep_awake_marker: str
    source_digest_script: str
    orphan_script: str
    source_state_file: str
    source_snapshot_dir: str
    colima: str
    bootstrap_script: str
    tart_readiness_script: str
    doctor_skips: dict[str, str]
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
    uv_lock: str
    tag_prefix: str
    stamped: tuple[StampedFile, ...]


class DoctorConfig(Strict):
    common_script: str

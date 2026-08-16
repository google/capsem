"""What `config/gate.toml` says about the product being installed and shipped.

Split out of `configschema`, which had grown to carry both the shapes the
harness needs to describe *itself* -- architectures, storage phases, pidfile
policy, the environment protocol -- and the shapes describing what the
machinery produces and installs. The seam is the same one `buildschema` sits
on: whether a section is about the machinery or about what it makes.

`configschema` keeps `Strict` and the harness-facing families, and this holds
the install layout, the guest the proof runs as, the package rail, the asset
manifest, and the three files a bootable tree is made of.
"""

from __future__ import annotations

from enum import StrEnum
from pathlib import PurePosixPath
from typing import Literal

from pydantic import model_validator

from capsem.dockerpolicy import BuildNetwork, ContainerNetwork

from .configschema import Strict


class MacosPlatform(Strict):
    """The macOS floor, and the name users see for it."""

    minimum_version: str
    minimum_release_name: str


class LinuxDistribution(Strict):
    """One distribution release for the glow-up proof to run the package on.

    The image is derived from `version` rather than restated beside it --
    `debian:13-slim` and `debian:trixie-slim` are the same digest, so naming
    the release twice only creates somewhere for the two to disagree.
    """

    name: str
    version: str
    repository: str
    tag_suffix: str = ""
    digest: str
    libc: str
    """The libc this release ships, e.g. `glibc 2.39` or `musl 1.2.5`.

    A fact about the distribution rather than about Capsem, recorded so the
    support claim can be derived without running anything -- badges and docs
    need an answer at read time.  The glow-up proof reads the real libc out of
    the image and refuses to pass if it disagrees, so this cannot quietly rot
    into a wrong claim the way a hand-written support list does.
    """

    def image(self) -> str:
        """The digest-pinned probe, e.g. `debian:13-slim@sha256:...`."""
        return f"{self.repository}:{self.version}{self.tag_suffix}@{self.digest}"

    def label(self) -> str:
        return f"{self.name} {self.version}"

    def flavour(self) -> str:
        return self.libc.split()[0]

    def libc_version(self) -> tuple[int, ...]:
        return tuple(int(part) for part in self.libc.split()[1].split("."))

    def sort_key(self) -> tuple[int, ...]:
        """Release order, numerically: `24.04` is below `26.04`, and `3.9`
        below `3.21` only when the parts are compared as numbers."""
        return tuple(int(part) for part in self.version.split("."))


class LinuxPlatform(Strict):
    """The Linux floor, expressed as the glibc the binaries are built against.

    `minimum_glibc` is not chosen; it is derived from the shipped binaries by
    `scripts/derive-deb-libc-floor.py` and recorded here so the support claim
    and the shipped bytes can be compared.
    """

    minimum_glibc: str
    distributions: tuple[LinuxDistribution, ...]
    """Every release the glow-up proof runs the package on.

    One list, not a supported set beside an unsupported one: whether a release
    is served follows from its libc and the floor, so splitting them would mean
    writing down an answer that is already derivable -- and a release could
    then sit in the wrong list and still pass.
    """

    def supported(self) -> tuple[LinuxDistribution, ...]:
        """The releases the published binaries actually run on."""
        floor = tuple(int(part) for part in self.minimum_glibc.split("."))
        return tuple(
            distribution
            for distribution in self.distributions
            if distribution.flavour() == "glibc" and distribution.libc_version() >= floor
        )

    def claims(self) -> tuple[str, ...]:
        """The support claim, one line per distribution, oldest served first.

        Derived rather than listed, so proving a newer release -- Ubuntu 26.04
        beside 24.04 -- never narrows what is promised.
        """
        oldest: dict[str, LinuxDistribution] = {}
        for distribution in self.supported():
            current = oldest.get(distribution.name)
            if current is None or distribution.sort_key() < current.sort_key():
                oldest[distribution.name] = distribution
        return tuple(
            f"{distribution.name} {distribution.version} or later"
            for distribution in oldest.values()
        )


class PlatformsConfig(Strict):
    """What the published product runs on.

    These floors were written out longhand in the two public `install.sh`
    copies, in the Tauri bundle, in the README and in the docs, with nothing
    comparing them -- and they had already drifted: the installers refused
    anything below macOS 14 while the app bundle advertised 13.0. A user-facing
    support claim is exactly the kind of value that must have one home.
    """

    macos: MacosPlatform
    linux: LinuxPlatform


class InstallLayout(Strict):
    assets: str
    config: str
    channel: str
    packages: str
    glowup: str
    glowup_evidence: str
    extra_owned_paths: tuple[str, ...]

    def owned_paths(self, mount: str) -> tuple[str, ...]:
        """Everything the container writes as its own user."""
        return tuple(
            f"{mount}/{path}"
            for path in (
                self.assets,
                self.config,
                self.channel,
                self.packages,
                self.glowup,
                self.glowup_evidence,
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
    serve_script: str
    sbom_script: str
    web_surface_script: str
    runtime_dependencies_script: str
    runtime_dependencies_config: str
    package_name: str


class AptSnapshotConfig(Strict):
    """The one immutable Ubuntu archive selected by every helper rail."""

    base: str
    id: str
    configure_script: str


class ProfileRevisionPolicy(StrEnum):
    """Whether a local graph authors new revisions or imports selected bytes."""

    STRICT = "strict"
    SELECTED_INPUT = "selected-input"


class InstallBuilderConfig(Strict):
    dockerfile: str
    tag_template: str
    source_tag_template: str
    source_identity_file: str
    identity_inputs: tuple[str, ...]
    identity_globs: tuple[str, ...]
    materialize_build_network: Literal[BuildNetwork.DEFAULT]
    source_build_network: Literal[BuildNetwork.NONE]
    cargo_store: str
    pnpm_store: str
    apt_packages: tuple[str, ...]


class InstallConfig(Strict):
    builder: InstallBuilderConfig
    container: str
    generated_inputs: tuple[str, ...]
    image: str
    dockerfile: str
    context: str
    smoke_network: Literal[ContainerNetwork.NONE]
    venv: str
    source_cli: str
    mount: str
    runtime_network: Literal[ContainerNetwork.NONE]
    channel: str
    profile_revision_policy: ProfileRevisionPolicy
    manifest_version: str
    systemd_ready_attempts: int
    systemd_ready_interval_seconds: float
    serve_ready_file: str
    serve_ready_attempts: int
    serve_ready_interval_seconds: float
    vm_devices: tuple[str, ...]
    optional_vm_devices: tuple[str, ...]
    vm_device_setup_script: str
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
    preinstall_root: str
    admin_relative: str
    request_script: str
    graph_manifest: str
    legacy_projection: str
    selected_inputs_dir: str
    proof_content_mount: str
    proof_assets_name: str
    proof_config_name: str
    package_runtime_packages: tuple[str, ...]
    layout: InstallLayout
    guest_user: GuestUser
    suite: InstallSuite

    @model_validator(mode="after")
    def _sealed_input_authority_is_consistent(self) -> InstallConfig:
        runtime = self.package_runtime_packages
        if not runtime or len(runtime) != len(set(runtime)):
            raise ValueError("install package_runtime_packages must be non-empty and unique")
        missing = tuple(name for name in runtime if name not in self.builder.apt_packages)
        if missing:
            raise ValueError(
                "install helper apt_packages omit package runtime dependencies: "
                + ", ".join(missing)
            )
        inputs = self.selected_inputs_dir
        if not inputs or inputs.startswith("/") or ".." in inputs.split("/"):
            raise ValueError("install selected_inputs_dir must stay beneath its content root")
        source_cli = PurePosixPath(self.source_cli)
        if not source_cli.is_absolute() or source_cli == PurePosixPath(self.installed_capsem):
            raise ValueError(
                "install source_cli must be an absolute path distinct from installed_capsem"
            )
        return self

    @property
    def preinstall_admin(self) -> str:
        return f"{self.preinstall_root}/{self.admin_relative}"

    @property
    def venv_python(self) -> str:
        """The exact interpreter materialized inside the sealed helper."""
        return str(PurePosixPath(self.venv) / "bin" / "python")

    @property
    def proof_assets_mount(self) -> str:
        return str(PurePosixPath(self.proof_content_mount) / self.proof_assets_name)

    @property
    def proof_config_mount(self) -> str:
        return str(PurePosixPath(self.proof_content_mount) / self.proof_config_name)


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
        return tuple(name for name in self.binaries if name not in self.binaries_without_version)


class PackageSigningConfig(Strict):
    """Where the local Tauri signing material lives, and how it is exported."""

    directory: str
    key: str
    password: str
    key_variable: str
    password_variable: str


class PackageBuilderConfig(Strict):
    dockerfile: str
    tag_template: str
    identity_inputs: tuple[str, ...]
    identity_globs: tuple[str, ...]
    materialize_build_network: Literal[BuildNetwork.DEFAULT]
    source_build_network: Literal[BuildNetwork.NONE]
    runtime_network: Literal[ContainerNetwork.NONE]
    cargo_store: str
    pnpm_store: str
    ort_lib_location: str


class PackageConfig(Strict):
    builder: PackageBuilderConfig
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
    lane_container: str
    generated_inputs: tuple[str, ...]
    lane_dockerfile: str
    lane_image: str
    writable_paths: tuple[str, ...]
    container_output_dir: str
    container_output_contents: str
    dist_dir: str
    proof: PackageProof

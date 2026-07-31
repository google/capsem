"""Loading `config/gate.toml` into validated objects.

Every value the gate works from arrives through here. Modules hold decisions;
this holds what those decisions are made from, so a container name or a
boundary/rail pair exists once and is read by the code, the contract tests, and
whoever is deciding whether to change it.

Validation is the point of using Pydantic rather than the raw `dict` tomllib
returns. A missing key, a mistyped timeout, or a storage phase naming a rail
that does not exist fails at load with the field named -- instead of surfacing
forty minutes in as a `KeyError` inside a Docker call.
"""

from __future__ import annotations

import tomllib
from functools import lru_cache
from pathlib import Path

from pydantic import BaseModel, ConfigDict, Field, ValidationError, model_validator

from . import host
from .errors import GateError

CONFIG_RELATIVE = Path("config") / "gate.toml"


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

    @property
    def pkg_config_path(self) -> str:
        """Where the cross toolchain's `.pc` files live inside the builder."""
        return f"/usr/lib/{self.gnu}/pkgconfig:/usr/share/pkgconfig"


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


class PackageConfig(Strict):
    builder_image: str
    build_script: str
    proof_selector: str
    default_manifest_url: str
    channels: tuple[str, ...]
    target_volume: str
    volumes: tuple[VolumeSpec, ...]

    def target_volume_for(self, arch: str) -> str:
        return self.target_volume.format(arch=arch)


class AssetsConfig(Strict):
    test_root: str
    profiles_glob: str
    required_artifacts: tuple[str, ...]
    failure_tail_lines: int
    shell_proof_timeout_seconds: int
    run_dir_template: str


class StampedFile(Strict):
    path: str
    kind: str
    key: str


class VersionsConfig(Strict):
    stamped: tuple[StampedFile, ...]


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


class GateConfig(Strict):
    version: int
    architectures: dict[str, Arch]
    storage: StorageConfig
    pidfiles: PidfileConfig
    install: InstallConfig
    package: PackageConfig
    assets: AssetsConfig
    versions: VersionsConfig
    lint: LintConfig
    boundary: BoundaryConfig

    root: Path = Field(exclude=True)
    """The checkout this configuration was loaded from."""

    @model_validator(mode="after")
    def _name_architectures(self) -> GateConfig:
        """Give each architecture the table key that names it."""
        for key, arch in self.architectures.items():
            if not arch.name:
                object.__setattr__(arch, "name", key)
        return self

    def path(self, relative: str) -> Path:
        return self.root / relative

    # -- architectures -----------------------------------------------------

    @property
    def _arch_aliases(self) -> dict[str, Arch]:
        return {
            alias.lower(): arch
            for arch in self.architectures.values()
            for alias in arch.aliases
        }

    def arch(self, spelling: str) -> Arch:
        """The architecture named by any accepted spelling of it."""
        table = self._arch_aliases
        try:
            return table[spelling.strip().lower()]
        except KeyError:
            raise GateError(
                f"unsupported architecture {spelling!r}; "
                f"expected one of {', '.join(sorted(table))}"
            ) from None

    def host_arch(self) -> Arch:
        """The architecture of the machine running the gate."""
        return self.arch(host.machine())


def load(root: Path) -> GateConfig:
    """Read and validate `config/gate.toml` for a checkout."""
    source = Path(root) / CONFIG_RELATIVE
    try:
        raw = tomllib.loads(source.read_text(encoding="utf-8"))
    except OSError as error:
        raise GateError(f"cannot read gate configuration: {error}") from None
    except tomllib.TOMLDecodeError as error:
        raise GateError(f"{source} is not valid TOML: {error}") from None

    try:
        return GateConfig(root=Path(root), **raw)
    except ValidationError as error:
        raise GateError(f"{source} is invalid:\n{error}") from None


@lru_cache(maxsize=4)
def _cached(root: Path) -> GateConfig:
    return load(root)


def for_root(root: Path) -> GateConfig:
    """The configuration for a checkout, parsed once per process."""
    return _cached(Path(root).resolve())

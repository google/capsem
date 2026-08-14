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
from typing import Literal

from pydantic import Field, ValidationError, model_validator

from . import host
from .assetschema import ArtifactsConfig, AssetsConfig
from .buildschema import (
    AuditsConfig,
    DevLoopConfig,
    FrontendConfig,
    FunctionalConfig,
    HostImageConfig,
    ImageBuildConfig,
    InitrdConfig,
    LogsConfig,
    ModulesConfig,
    ReleaseConfig,
    SbomConfig,
    ServiceConfig,
    SigningConfig,
    SmokeConfig,
    SuitesConfig,
    WebSurfacesConfig,
)
from .configschema import (
    Arch,
    CandidateConfig,
    DoctorConfig,
    EnvironmentConfig,
    PidfileConfig,
    StorageConfig,
    Strict,
    VersionsConfig,
)
from .digestschema import BenchmarkRegressionConfig
from .errors import GateError
from .harnessschema import (
    BoundaryConfig,
    DiskConfig,
    Exclusive,
    ExecutionConfig,
    PrefixConfig,
    RunLogConfig,
    SandboxConfig,
    WorkspaceConfig,
)
from .lintschema import LintConfig, LintSurface
from .lockschema import LocksConfig
from .productschema import (
    AptSnapshotConfig,
    InstallConfig,
    PackageConfig,
)
from .toolchainschema import ToolchainConfig

CONFIG_RELATIVE = Path("config") / "gate.toml"


class GateConfig(Strict):
    version: Literal[1]
    """The schema this code understands, and only that.

    A bare `int` accepted a file written for a later schema: it loaded happily
    and was then read with the wrong meaning, which is worse than refusing it.
    """
    architectures: dict[str, Arch]
    apt_snapshot: AptSnapshotConfig
    storage: StorageConfig
    pidfiles: PidfileConfig
    install: InstallConfig
    environment: EnvironmentConfig
    artifacts: ArtifactsConfig
    package: PackageConfig
    assets: AssetsConfig
    candidate: CandidateConfig
    versions: VersionsConfig
    doctor: DoctorConfig
    lint: LintConfig
    boundary: BoundaryConfig
    lint_surfaces: tuple[LintSurface, ...]
    execution: ExecutionConfig
    locks: LocksConfig
    runlog: RunLogConfig
    benchmark_regression: BenchmarkRegressionConfig
    disk: DiskConfig
    prefix: PrefixConfig
    sandbox: SandboxConfig
    workspace: WorkspaceConfig
    service: ServiceConfig
    smoke: SmokeConfig
    initrd: InitrdConfig
    release: ReleaseConfig
    devloop: DevLoopConfig
    audits: AuditsConfig
    websurfaces: WebSurfacesConfig
    suites: SuitesConfig
    toolchain: ToolchainConfig
    functional: FunctionalConfig
    modules: ModulesConfig
    imagebuild: ImageBuildConfig
    hostimage: HostImageConfig
    sbom: SbomConfig
    signing: SigningConfig
    frontend: FrontendConfig
    logs: LogsConfig

    root: Path = Field(exclude=True)
    """The checkout this configuration was loaded from."""

    pkg_config_template: str

    @model_validator(mode="after")
    def _name_architectures(self) -> GateConfig:
        """Give each architecture the table key and template that name it."""
        for key, arch in self.architectures.items():
            if not arch.name:
                object.__setattr__(arch, "name", key)
            if not arch.pkg_config_template:
                object.__setattr__(arch, "pkg_config_template", self.pkg_config_template)
        required_ort = {arch.rust_target for arch in self.architectures.values()}
        missing_ort = sorted(required_ort - set(self.toolchain.ort.distributions))
        if missing_ort:
            raise ValueError(
                "package target ORT distributions are missing: " + ", ".join(missing_ort)
            )
        return self

    def path(self, relative: str) -> Path:
        return self.root / relative

    # -- architectures -----------------------------------------------------

    @property
    def _arch_aliases(self) -> dict[str, Arch]:
        return {
            alias.lower(): arch for arch in self.architectures.values() for alias in arch.aliases
        }

    def arch(self, spelling: str) -> Arch:
        """The architecture named by any accepted spelling of it."""
        table = self._arch_aliases
        try:
            return table[spelling.strip().lower()]
        except KeyError:
            raise GateError(
                f"unsupported architecture {spelling!r}; expected one of {', '.join(sorted(table))}"
            ) from None

    def host_arch(self) -> Arch:
        """The architecture of the machine running the gate."""
        return self.arch(host.machine())

    # -- contention --------------------------------------------------------

    def shared(self, name: str) -> Exclusive:
        """A claim that admits other shared claims, and excludes writers.

        Looked up through `exclusive`, so an invented name is refused here for
        the same reason: a claim on nothing contends with nothing.
        """
        return self.exclusive(name).held_shared()

    def exclusive(self, name: str) -> Exclusive:
        """The named thing only one step may hold at a time.

        Looked up rather than constructed, for the same reason architectures
        are: a step that invents its own exclusive contends with nothing, and
        would run beside the step it was written to avoid.
        """
        try:
            return self.execution.exclusives[name]
        except KeyError:
            raise GateError(
                f"unknown exclusive {name!r}; declare it in "
                f"[execution.exclusives] with a reason, or use one of "
                f"{', '.join(sorted(self.execution.exclusives))}"
            ) from None


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

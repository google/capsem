"""What `config/gate.toml` says about building and releasing the product.

Split from `harnessschema`, which describes the gate running itself. The seam
is whether a section is about the machinery or about what the machinery makes;
one file carrying both was past the module ceiling this package enforces on its
own source.
"""

from __future__ import annotations

from pathlib import PurePosixPath
from typing import Annotated, Literal

from pydantic import PositiveInt, StringConstraints, model_validator

from ..policy.dockerpolicy import BuildNetwork, ContainerNetwork
from .configschema import Strict
from .qualifyschema import AuditsConfig as AuditsConfig
from .qualifyschema import DependencyAuditConfig as DependencyAuditConfig
from .qualifyschema import FunctionalConfig as FunctionalConfig
from .qualifyschema import GreyjoyConfig as GreyjoyConfig
from .qualifyschema import KingslandingConfig as KingslandingConfig
from .qualifyschema import ModulesConfig as ModulesConfig
from .qualifyschema import PinnedImageConfig as PinnedImageConfig
from .qualifyschema import QualificationConfig as QualificationConfig


class SourcePackageConfig(Strict):
    project: str
    manifest: str
    source: str
    tests: str
    build_output: str


class SdkConfig(SourcePackageConfig):
    specification: str


class PythonSdkConfig(SdkConfig):
    environment_python: str
    package_build_requirements: str


class NodePackageConfig(Strict):
    project: str


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
    linux_packages_glob: str
    macos_package_name: str
    expected_debs: int
    spdx_version: str


class SigningConfig(Strict):
    entitlements: str
    binaries: tuple[str, ...]
    built: tuple[str, ...]
    built_elsewhere: tuple[str, ...]
    guest_crate: str
    release_binary: str
    build_timeout_seconds: PositiveInt
    sign_timeout_seconds: PositiveInt

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
    workspace: str
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
    config_root: str
    output: str
    doctor_skips: dict[str, str]

    @model_validator(mode="after")
    def _workspace_is_arch_scoped(self) -> ImageBuildConfig:
        if "{arch}" not in self.workspace_root:
            raise ValueError("image workspace_root must contain {arch}")
        rendered = self.workspace_root.format(arch="arch")
        path = PurePosixPath(rendered)
        if path.is_absolute() or ".." in path.parts:
            raise ValueError("image workspace_root must remain inside the checkout")
        guest = PurePosixPath(self.workspace_guest_dir)
        if guest.is_absolute() or len(guest.parts) != 1 or guest.name in {"", ".", ".."}:
            raise ValueError("image workspace_guest_dir must be one relative directory")
        return self


class WebSurfacesConfig(Strict):
    script: str
    targets: tuple[str, ...]
    blocks_clippy: str
    needs_generated_settings: str
    building: tuple[str, ...]
    """Which surfaces run a bundler, and so which declare `COMPILE` and take
    the Astro claim. Read per target rather than assumed of all of them: one
    declaration written for a list of four outlived the build it described."""


class InitrdConfig(Strict):
    binaries: tuple[str, ...]
    staging: str
    build: tuple[str, ...]
    lint_packages: tuple[str, ...]
    lint_features: tuple[str, ...]
    """The guest feature set, linted on the host with every target. Workspace
    clippy compiles the host feature set, and the guest builder only runs
    `cargo build`, so between them nothing ever compiled a guest test target:
    `capsem-bench --features guest` had uncompilable tests for as long as the
    field they named was host-only."""
    init: str
    files: tuple[str, ...]
    trees: tuple[str, ...]
    prune: str
    binary_mode: int
    init_mode: int
    manifest: tuple[str, ...]
    hash_assets: str


class DevLoopConfig(Strict):
    setup_sentinel: str
    dev_lock: str
    tauri: tuple[str, ...]
    frontend_dev: tuple[str, ...]
    frontend_dir: str
    tui: tuple[str, ...]
    surfaces: tuple[str, ...]
    rust_affected: str
    generate_settings: str
    generated_settings_scratch: str
    check_settings: str

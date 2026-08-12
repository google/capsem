"""Exact container bases for guest image builds.

The Docker daemon is the declared container-fetch boundary.  The gate itself
stays inside its kernel sandbox, asks the daemon for immutable child manifests,
then runs every image build from those locally materialized inputs.
"""

from __future__ import annotations

from collections.abc import Iterable

from capsem.builder import assettools, guestbuilder
from capsem.builder.config import load_guest_config
from capsem.builder.guestbuilder import build_arguments, image_repository, image_tag
from capsem.builder.models import ArchConfig, BuildConfig

from .actions import Action
from .config import GateConfig
from .context import Context
from .docker import Docker
from .errors import GateError
from .imageidentity import require_input_key
from .proc import Runner
from .storage import Storage


def build_config(config: GateConfig) -> BuildConfig:
    """Load the profile-materialization source through its product schema."""
    build = load_guest_config(config.path(config.imagebuild.source_config)).build
    missing = sorted(set(config.architectures) - set(build.architectures))
    if missing:
        raise GateError(f"guest image config has no base for: {', '.join(missing)}")
    for name, gate_arch in config.architectures.items():
        platform = build.architectures[name].docker_platform
        platform_arch = platform.rpartition("/")[2]
        if platform_arch != gate_arch.dpkg:
            raise GateError(
                f"guest image architecture {name} uses {platform}, expected the "
                f"configured {gate_arch.dpkg} architecture"
            )
    return build


def selected(
    config: GateConfig, names: Iterable[str] | None = None
) -> tuple[tuple[str, ArchConfig], ...]:
    """Return validated base inputs in deterministic architecture order."""
    build = build_config(config)
    wanted = tuple(names) if names is not None else tuple(config.architectures)
    unknown = sorted(set(wanted) - set(build.architectures))
    if unknown:
        raise GateError(f"guest image config has no base for: {', '.join(unknown)}")
    return tuple((name, build.architectures[name]) for name in wanted)


def required_rust_builder_names(
    config: GateConfig,
    names: Iterable[str] | None = None,
) -> tuple[str, ...]:
    """Every architecture uses the same locked/offline container helper."""
    return tuple(name for name, _arch in selected(config, names))


def prefetch(
    runner: Runner,
    config: GateConfig,
    names: Iterable[str] | None = None,
    *,
    rust_names: Iterable[str] | None = None,
    asset_tools: bool = False,
) -> None:
    """Pull each absent exact base through the Docker daemon fetch edge."""
    docker = Docker(runner)
    base_names = [name for name, _arch in selected(config, names)]
    if asset_tools and config.host_arch().name not in base_names:
        base_names.append(config.host_arch().name)
    for name, arch in selected(config, base_names):
        if docker.image_exists(arch.base_image, platform=arch.docker_platform):
            runner.note(f"exact {name} guest base is already present: {arch.base_image}")
        else:
            docker.pull(arch.base_image, platform=arch.docker_platform)

    # The Rust builder base is the *host* platform's exact child even for a
    # foreign target, because a foreign target is cross-compiled rather than
    # emulated. Two requested architectures therefore normally resolve to one
    # pull, which `image_exists` collapses on the second.
    build = build_config(config)
    rust_scope = names if rust_names is None else rust_names
    for name, _arch in selected(config, rust_scope):
        resolved = guestbuilder.environment(build, name)
        if docker.image_exists(resolved.base_image, platform=resolved.docker_platform):
            runner.note(f"exact {name} Rust builder base is already present: {resolved.base_image}")
        else:
            docker.pull(
                resolved.base_image,
                platform=resolved.docker_platform,
            )


def materialize_asset_tools(runner: Runner, config: GateConfig) -> None:
    """Build the input-keyed host helper used by the sealed asset tail."""
    docker = Docker(runner)
    storage = Storage(runner)
    build = build_config(config)
    name = config.host_arch().name
    arch = build.architectures[name]
    if not docker.image_exists(arch.base_image, platform=arch.docker_platform):
        raise GateError(
            f"exact {name} asset-tool base is missing: {arch.base_image}; "
            "run guest base prefetch first"
        )
    tag = assettools.image_tag(build, name, config.root)
    if docker.image_exists(tag, platform=arch.docker_platform):
        require_input_key(
            docker,
            tag,
            label=assettools.INPUT_KEY_LABEL,
            subject="asset tools helper",
        )
        runner.note(f"locked {name} asset tools are already present: {tag}")
    else:
        docker.build(
            tag=tag,
            dockerfile=config.path(build.asset_tools.dockerfile).as_posix(),
            context=str(config.root),
            args=assettools.build_arguments(build, name, tag),
            platform=arch.docker_platform,
            network=build.asset_tools.materialize_network,
        )
        require_input_key(
            docker,
            tag,
            label=assettools.INPUT_KEY_LABEL,
            subject="asset tools helper",
        )
    exact_id = docker.image_id(tag, platform=arch.docker_platform)
    runner.note(f"sealed asset tools identity: {tag} -> {exact_id}")
    storage.reclaim(assettools.image_repository(build, name), keep=tag)


def materialize_rust_builders(
    runner: Runner,
    config: GateConfig,
    names: Iterable[str] | None = None,
) -> None:
    """Build lockfile-keyed helpers after cross-platform execution is proven."""
    docker = Docker(runner)
    storage = Storage(runner)
    build = build_config(config)
    for name, _arch in selected(config, names):
        resolved = guestbuilder.environment(build, name)
        if not docker.image_exists(
            resolved.base_image,
            platform=resolved.docker_platform,
        ):
            raise GateError(
                f"exact {name} Rust builder base is missing: "
                f"{resolved.base_image}; run guest base prefetch first"
            )
        builder = image_tag(build, name, config.root)
        if docker.image_exists(builder, platform=resolved.docker_platform):
            runner.note(f"locked {name} guest Rust builder is already present: {builder}")
        else:
            docker.build(
                tag=builder,
                dockerfile=config.path(build.guest_rust_builder.dockerfile).as_posix(),
                context=str(config.root),
                args=build_arguments(resolved),
                platform=resolved.docker_platform,
                network=build.materialize_network,
            )
        exact_id = docker.image_id(builder, platform=resolved.docker_platform)
        runner.note(f"sealed {name} guest Rust builder identity: {builder} -> {exact_id}")
        storage.reclaim(image_repository(build, name), keep=builder)


class Prefetch(Action, name="guest-base-prefetch"):
    """A visible, timed cold-host boundary shared by every asset rail."""

    def __init__(
        self,
        names: Iterable[str] | None = None,
        *,
        rust_names: Iterable[str] | None = None,
        asset_tools: bool = False,
    ) -> None:
        self._names = tuple(names) if names is not None else None
        self._rust_names = tuple(rust_names) if rust_names is not None else None
        self._asset_tools = asset_tools

    def render(self) -> str:
        scope = "all architectures" if self._names is None else ", ".join(self._names)
        rust_names = self._names if self._rust_names is None else self._rust_names
        rust_scope = "all architectures" if rust_names is None else ", ".join(rust_names) or "none"
        return (
            f"materialize exact guest base images ({scope}); "
            f"Rust builder bases ({rust_scope}); "
            f"asset tools ({'host' if self._asset_tools else 'none'})"
        )

    def perform(self, context: Context) -> None:
        prefetch(
            context.runner,
            context.config,
            self._names,
            rust_names=self._rust_names,
            asset_tools=self._asset_tools,
        )


class MaterializeRustBuilders(Action, name="guest-rust-builder-materialize"):
    """Build the locked helper images at the preflight's network-open edge."""

    def __init__(self, names: Iterable[str] | None = None) -> None:
        self._names = tuple(names) if names is not None else None

    def render(self) -> str:
        scope = "all architectures" if self._names is None else ", ".join(self._names)
        return f"materialize locked guest Rust builders ({scope})"

    def perform(self, context: Context) -> None:
        materialize_rust_builders(context.runner, context.config, self._names)


class MaterializeAssetTools(Action, name="asset-tools-materialize"):
    """Build the one host-native helper before publishable post-processing."""

    def render(self) -> str:
        return "materialize locked host EROFS and CycloneDX tools"

    def perform(self, context: Context) -> None:
        materialize_asset_tools(context.runner, context.config)

"""One content-addressed guest-binary generation shared by every consumer."""

from __future__ import annotations

from collections.abc import Callable, Sequence
from pathlib import Path

from . import componentcache, guestbuilder
from .models import BuildConfig

Compiler = Callable[[BuildConfig, str, Path, Path], list[Path]]


def identity(
    build: BuildConfig,
    arch_name: str,
    repository: Path,
    binary_names: tuple[str, ...],
) -> str:
    """Bind guest bytes to their source, sealed builder, and output contract."""
    if not binary_names or len(binary_names) != len(set(binary_names)):
        raise ValueError("guest binary names must be non-empty and unique")
    return componentcache.input_digest(
        {
            "arch": arch_name,
            "builder": guestbuilder.image_tag(build, arch_name, repository),
            "outputs": binary_names,
            "source": componentcache.source_digest(
                repository, build.guest_rust_builder.source_roots
            ),
        }
    )


def _ordered(paths: Sequence[Path], names: tuple[str, ...]) -> tuple[Path, ...]:
    by_name = {path.name: path for path in paths}
    if len(by_name) != len(paths) or set(by_name) != set(names):
        raise ValueError("guest binary producer did not return the exact configured outputs")
    return tuple(by_name[name] for name in names)


def current(
    build: BuildConfig,
    arch_name: str,
    repository: Path,
    output: Path,
    binary_names: tuple[str, ...],
) -> bool:
    """Whether staging exactly matches the current content-addressed generation."""
    generation = identity(build, arch_name, repository, binary_names)
    found = componentcache.current(repository, "guest-binaries", generation, output)
    return found is not None and bool(_ordered(found, binary_names))


def materialize(
    build: BuildConfig,
    arch_name: str,
    repository: Path,
    output: Path,
    binary_names: tuple[str, ...],
    compiler: Compiler,
) -> tuple[Path, ...]:
    """Restore one verified generation, compiling and publishing only on miss."""
    generation = identity(build, arch_name, repository, binary_names)
    restored = componentcache.restore(repository, "guest-binaries", generation, output)
    if restored is not None:
        return _ordered(restored, binary_names)

    compiled = _ordered(compiler(build, arch_name, repository, output), binary_names)
    componentcache.store(repository, "guest-binaries", generation, output, binary_names)
    return compiled

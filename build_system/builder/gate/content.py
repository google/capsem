"""One verified profile-content bundle consumed by later gate rails.

Assets and materialized configuration are a pair.  Keeping both paths relative
to one root makes it impossible for a caller to combine an IronBank-proved
asset tree with stale configuration from the checkout.  Construction is pure;
the filesystem proof is an explicit run action.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from .errors import GateError


def _relative(path: Path) -> bool:
    return not path.is_absolute() and ".." not in path.parts


def _require_real_subdirectory(root: Path, relative: Path, label: str) -> Path:
    """Refuse every symlink component before returning a mount source."""
    current = root
    for component in relative.parts:
        current /= component
        if current.is_symlink():
            raise GateError(f"profile content {label} path contains a symlink: {current}")
    if not current.is_dir():
        raise GateError(f"profile content {label} are missing: {current}")
    return current


@dataclass(frozen=True)
class ProfileContent:
    root: Path
    assets_path: Path
    config_path: Path

    def __post_init__(self) -> None:
        for path in (self.assets_path, self.config_path):
            if not _relative(path):
                raise ValueError(f"ProfileContent requires a relative path under its root: {path}")

    @classmethod
    def isolated(cls, config, root: Path) -> ProfileContent:
        """The private per-profile layout produced and proved by AssetGate."""
        return cls(
            Path(root),
            Path(config.assets.merged_assets_dir),
            Path(config.assets.merged_config_dir),
        )

    @classmethod
    def built_profile(cls, config, profile: str) -> ProfileContent:
        """One real per-profile bundle built below the gate's test root."""
        return cls.isolated(config, config.path(config.assets.test_root) / profile)

    @classmethod
    def standalone(cls, config) -> ProfileContent:
        """The checkout layout accepted only by the public standalone rail."""
        return cls(
            config.root,
            Path(config.functional.assets_dir),
            Path(config.functional.config_root),
        )

    @classmethod
    def staged(cls, config, root: Path) -> ProfileContent:
        """The standalone layout, anchored at a lane's workspace.

        Same relative shape, different root. A release lane stages its cohort
        into the workspace and then qualifies from a private prefix carrying
        only tracked files, so the checkout anchor names a directory nothing
        ever wrote. Absolute, because the whole point is to leave the prefix.
        """
        if not root.is_absolute():
            raise ValueError("a staged content root must be absolute")
        return cls(
            root,
            Path(config.functional.assets_dir),
            Path(config.functional.config_root),
        )

    @property
    def assets(self) -> Path:
        return self.root / self.assets_path

    @property
    def config(self) -> Path:
        return self.root / self.config_path

    def profiles(self, config) -> Path:
        return self.config / config.functional.profiles_subdir

    def config_manifest(self, config) -> Path:
        """The runtime manifest copied into this content bundle's config tree."""
        return self.config / config.assets.merged_assets_dir / config.install.manifest_name

    def require_complete(self, config, arches: tuple | None = None) -> None:
        """Fail unless this exact pair is complete for the requested targets."""
        requested = tuple(config.architectures.values()) if arches is None else arches
        for arch in requested:
            if config.architectures.get(arch.name) != arch:
                raise GateError(f"content target {arch.name!r} is not a configured architecture")

        if self.root.is_symlink():
            raise GateError(f"profile content root is a symlink: {self.root}")
        if not self.root.is_dir():
            raise GateError(f"profile content root is missing: {self.root}")
        assets = _require_real_subdirectory(self.root, self.assets_path, "assets")
        _require_real_subdirectory(self.root, self.config_path, "config")

        manifest = assets / config.install.manifest_name
        config_manifest = self.config_manifest(config)
        if not manifest.is_file():
            raise GateError(f"profile content asset manifest is missing: {manifest}")
        if not config_manifest.is_file():
            raise GateError(f"profile content config manifest is missing: {config_manifest}")
        manifest_bytes = manifest.read_bytes()
        if config_manifest.read_bytes() != manifest_bytes:
            raise GateError(
                f"profile content config manifest {config_manifest} does not match {manifest}"
            )

        declared = _declared_arches(manifest, manifest_bytes)
        for arch in requested:
            if arch.name not in declared:
                raise GateError(
                    f"profile content manifest does not declare {arch.name}: {manifest}"
                )
            directory = assets / arch.name
            if not directory.is_dir():
                raise GateError(f"profile content assets are missing {arch.name}: {directory}")
            for name in (*config.artifacts.bootable, *config.assets.evidence_artifacts):
                artifact = directory / name
                if not artifact.is_file():
                    raise GateError(
                        f"profile content artifact is missing {arch.name}/{name}: {artifact}"
                    )

        profiles = self.profiles(config)
        if not profiles.is_dir():
            raise GateError(f"profile content catalog is missing: {profiles}")
        if not any(path.is_file() for path in profiles.glob("*/profile.toml")):
            raise GateError(f"profile content catalog has no materialized profiles: {profiles}")


@dataclass(frozen=True)
class LocalInstallContent:
    """Fresh local content whose checked graph is authored during install."""

    content: ProfileContent


@dataclass(frozen=True)
class SelectedInstallContent:
    """Manifest-selected content whose packaged channel remains authoritative."""

    content: ProfileContent

    def inputs(self, config) -> Path:
        return self.content.root / config.install.selected_inputs_dir

    def require_complete(self, config, *, arches: tuple) -> None:
        """Prove the paired projection and require its verified source graph."""
        self.content.require_complete(config, arches=arches)
        relative = Path(config.install.selected_inputs_dir)
        if not _relative(relative):
            raise GateError("selected install input directory must be relative to content root")
        inputs = _require_real_subdirectory(self.content.root, relative, "selected release inputs")
        for name in (config.package.release_inputs_name, config.install.manifest_name):
            if not (inputs / name).is_file():
                raise GateError(f"selected release input is missing: {inputs / name}")

        # Keep the fetched graph byte-for-byte. `release-inputs.json` binds its
        # public URLs to safe local paths and immutable digests; the shared
        # verifier checks that report on the host and again inside the sealed
        # container before any byte is consumed. Profile staging rewrites a
        # separate runtime projection, so requiring generated file:// URLs in
        # this source graph rejects the real hosted channel.
        manifest = inputs / config.install.manifest_name
        try:
            document = json.loads(manifest.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise GateError(f"selected install manifest is invalid: {manifest}: {error}") from None
        if not isinstance(document, dict):
            raise GateError(f"selected install manifest is not a JSON object: {manifest}")


InstallContent = LocalInstallContent | SelectedInstallContent


def _declared_arches(path: Path, payload: bytes) -> frozenset[str]:
    try:
        manifest = json.loads(payload)
        assets = manifest.get("assets", {})
        current = assets.get("current")
        releases = assets.get("releases", {})
        if current is not None:
            arches = releases[current]["arches"]
            if not isinstance(arches, dict):
                raise TypeError("arches is not a table")
            names = frozenset(name for name in arches if isinstance(name, str))
            if len(names) != len(arches):
                raise TypeError("arches has a non-string name")
            return names

        profiles = manifest["profiles"]
        return frozenset(
            entry["architecture"]
            for profile in profiles.values()
            for entry in profile["architectures"]
        )
    except (json.JSONDecodeError, KeyError, TypeError, AttributeError) as error:
        raise GateError(
            f"profile content manifest has no valid architecture graph: {path}: {error}"
        ) from None

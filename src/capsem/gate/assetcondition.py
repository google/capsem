"""Defer the warm asset shortcut until the invariant graph executes."""

from __future__ import annotations

import json
from threading import Lock

from capsem_builder.release.obom import validate_exported_rootfs_obom

from .actions import Action
from .config import Arch, GateConfig
from .context import Context
from .filesystem import digest_of


def _current_arch_entries(config: GateConfig, arch: Arch) -> dict | None:
    manifest = config.path(config.imagebuild.output) / config.install.manifest_name
    try:
        document = json.loads(manifest.read_text(encoding="utf-8"))
        if document.get("format") != 2:
            return None
        assets = document["assets"]
        current = assets["current"]
        entries = assets["releases"][current]["arches"][arch.name]
        return entries if isinstance(entries, dict) else None
    except (OSError, json.JSONDecodeError, KeyError, TypeError, AttributeError):
        return None


def missing(config: GateConfig, arch: Arch) -> list[str]:
    """Incomplete or mutated outputs from the final successful producer."""
    tree = config.path(config.imagebuild.output) / arch.name
    required = (*config.artifacts.bootable, *config.assets.evidence_artifacts)
    entries = _current_arch_entries(config, arch)
    if entries is None:
        return [config.install.manifest_name, *required]

    incomplete: list[str] = []
    for name in required:
        path = tree / name
        entry = entries.get(name)
        if not isinstance(entry, dict):
            incomplete.append(name)
            continue
        try:
            size = path.stat().st_size
            expected_size = entry["size"]
            expected_hash = entry["hash"]
            if (
                not path.is_file()
                or size == 0
                or type(expected_size) is not int
                or expected_size != size
                or not isinstance(expected_hash, str)
                or digest_of(path, algorithm="blake3") != expected_hash
            ):
                incomplete.append(name)
        except (OSError, KeyError, TypeError):
            incomplete.append(name)
    obom = tree / config.assets.obom_artifact
    if obom.name not in incomplete:
        try:
            validate_exported_rootfs_obom(obom, architecture=arch.name)
        except (OSError, UnicodeError, RuntimeError):
            incomplete.append(obom.name)
    return incomplete


class AssetRecovery:
    """One thread-safe warm/cold decision shared by a recovery cohort."""

    def __init__(self, config: GateConfig, arch: Arch) -> None:
        self._config = config
        self._arch = arch
        self._needed: bool | None = None
        self._lock = Lock()

    def needed(self) -> bool:
        with self._lock:
            if self._needed is None:
                self._needed = bool(missing(self._config, self._arch))
            return self._needed

    def when(self, action: Action) -> WhenAssetsMissing:
        return WhenAssetsMissing(self, action)


class WhenAssetsMissing(Action, name="when-assets-missing"):
    """Run an action only when the host asset cohort needs recovery.

    The predicate deliberately lives in ``perform``. Evaluating it in a plan
    constructor made the public checkout and its private prefix describe
    different labels, so a private failure could not be selected by ``--from``.
    """

    def __init__(self, recovery: AssetRecovery, action: Action) -> None:
        self._recovery = recovery
        self._action = action

    def render(self) -> str:
        return f"when host assets are missing: {self._action.render()}"

    def perform(self, context: Context) -> None:
        if self._recovery.needed():
            self._action.perform(context)

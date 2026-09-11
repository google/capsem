"""Defer the warm asset shortcut until the invariant graph executes.

Warm is not current. The shortcut used to ask one question -- are the host
assets there? -- so a kernel defconfig change never rebuilt the kernel under
`focus-test`: the run stayed green, booted the old kernel, and the run log
said nothing about assets at all. The identity the release lanes already
compute is what makes reuse honest, and the decision is noted once per run,
naming what changed, where the next reader looks.
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass
from threading import Lock

from ..release.obom import validate_exported_rootfs_obom
from . import assetidentity
from .actions import Action
from .config import Arch, GateConfig
from .context import Context
from .execution import Kind, Needs, Speed, Step, step
from .filesystem import digest_of, write_text


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


def record_path(config: GateConfig):
    return config.path(config.imagebuild.output) / config.assets.host_identity_record


def record_identity(config: GateConfig) -> str:
    """Write the checkout identity the host assets were just built from."""
    roots = assetidentity.roots(config)
    identity = assetidentity.lane_identity(config)
    document = {
        "identity": identity,
        "inputs": assetidentity.inputs(config.root, roots),
        "recorded_at": time.time(),
    }
    write_text(record_path(config), json.dumps(document, sort_keys=True) + "\n")
    return identity


def stale(config: GateConfig) -> str | None:
    """Why complete host assets may not be reused, or None when they may."""
    try:
        document = json.loads(record_path(config).read_text(encoding="utf-8"))
        recorded_identity = document["identity"]
        recorded_inputs = document["inputs"]
        if not isinstance(recorded_identity, str) or not isinstance(recorded_inputs, dict):
            raise TypeError("malformed host asset identity record")
    except (OSError, ValueError, KeyError, TypeError):
        return "have no identity record beside them, so they cannot prove they are current"
    identity = assetidentity.lane_identity(config)
    if identity == recorded_identity:
        return None
    current = assetidentity.inputs(config.root, assetidentity.roots(config))
    changed = sorted(
        path
        for path in set(recorded_inputs) | set(current)
        if recorded_inputs.get(path) != current.get(path)
    )
    shown = config.assets.host_identity_changed_inputs_shown
    named = ", ".join(changed[:shown])
    if len(changed) > shown:
        named += f", and {len(changed) - shown} more"
    return (
        f"are stale: identity {identity[:12]} differs from recorded "
        f"{recorded_identity[:12]}; changed inputs: {named}"
    )


@dataclass(frozen=True)
class Decision:
    needed: bool
    reason: str


class AssetRecovery:
    """One thread-safe warm/cold decision shared by a recovery cohort."""

    def __init__(self, config: GateConfig, arch: Arch) -> None:
        self._config = config
        self._arch = arch
        self._decision: Decision | None = None
        self._announced = False
        self._lock = Lock()

    def decision(self) -> Decision:
        with self._lock:
            if self._decision is None:
                self._decision = self._decide()
            return self._decision

    def _decide(self) -> Decision:
        gone = missing(self._config, self._arch)
        if gone:
            return Decision(
                True,
                f"host assets ({self._arch.name}) are missing or incomplete: "
                f"{', '.join(gone)}; rebuilding",
            )
        why = stale(self._config)
        if why is not None:
            return Decision(True, f"host assets ({self._arch.name}) {why}; rebuilding")
        identity = assetidentity.lane_identity(self._config)
        return Decision(
            False,
            f"host assets ({self._arch.name}) are current for identity {identity[:12]}; reusing",
        )

    def needed(self) -> bool:
        return self.decision().needed

    def announce(self, context: Context) -> None:
        """Note the decision once, the first time any action in the cohort runs."""
        decision = self.decision()
        with self._lock:
            if self._announced:
                return
            self._announced = True
        context.runner.note(decision.reason)

    def when(self, action: Action) -> WhenHostAssetsStale:
        return WhenHostAssetsStale(self, action)

    def record(self) -> RecordHostAssetIdentity:
        return RecordHostAssetIdentity(self._config)

    def record_step(self) -> Step:
        """The cohort's terminal step: what the rebuilt tree was built from.

        Skipped like every other cohort action when nothing was rebuilt, so a
        reused tree keeps the record that justified reusing it.
        """
        return step(
            "record-identity",
            self.when(self.record()),
            kind=Kind.PACKAGE,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        )


class WhenHostAssetsStale(Action, name="when-host-assets-stale"):
    """Run an action only when the host asset cohort needs recovery.

    The predicate deliberately lives in ``perform``. Evaluating it in a plan
    constructor made the public checkout and its private prefix describe
    different labels, so a private failure could not be selected by ``--from``.
    """

    def __init__(self, recovery: AssetRecovery, action: Action) -> None:
        self._recovery = recovery
        self._action = action

    def render(self) -> str:
        return f"when host assets are missing or stale: {self._action.render()}"

    def perform(self, context: Context) -> None:
        self._recovery.announce(context)
        if self._recovery.needed():
            self._action.perform(context)


class RecordHostAssetIdentity(Action, name="record-host-asset-identity"):
    """The terminal action of the last image build: what these assets are from."""

    def __init__(self, config: GateConfig) -> None:
        self._config = config

    def render(self) -> str:
        return "record host asset identity"

    def perform(self, context: Context) -> None:
        identity = record_identity(self._config)
        context.runner.note(f"recorded host asset identity {identity[:12]}")

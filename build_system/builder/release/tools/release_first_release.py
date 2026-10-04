#!/usr/bin/env python3
"""Release-owned classification of a channel's first public pairing.

A channel that has never served a working binary graph has no predecessor to
upgrade from. The lane already reaches that conclusion by itself: a retired
public graph resolves to `bootstrap`, and the projected public-before manifest
then declares no packages and no runtime. What was missing is that the
installed-product proof still demanded a predecessor package anyway, so the lane
refused the very release it had just classified as the first one -- which is how
this line's first binary release died on `expected one current Linux
amd64/x86_64 package, found 0`, after every build and signature had passed.

`FRESH_INSTALL` was always the transition for "nothing was installed before".
Deciding which pairing is that case lives here, in one place, so the classifier
and the validator cannot come to different conclusions about the same manifests.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from . import repository_root
from .release_glowup import (
    ArtifactIdentity,
    GlowupContractError,
    TransitionKind,
    artifact_identity_from_manifest_package,
    load_manifest_bytes,
    validate_pairing_inputs,
)

ROOT = repository_root()


def _runtime(manifest_bytes: bytes, label: str) -> object:
    runtime = load_manifest_bytes(manifest_bytes).get("runtime")
    if runtime is not None and not isinstance(runtime, dict):
        raise GlowupContractError(f"{label} manifest runtime must be an object or null")
    return runtime


def public_before_is_unpublished(before_manifest_bytes: bytes) -> bool:
    """Whether the public-before graph offers nothing a user could have installed.

    Both halves have to be empty. A graph with a runtime but no packages is not
    a first release -- it is a broken one, and calling it fresh would skip the
    upgrade proof for a channel that really does have a predecessor.
    """
    manifest = load_manifest_bytes(before_manifest_bytes)
    packages = manifest.get("packages")
    if not isinstance(packages, list):
        raise GlowupContractError("public-before manifest packages must be an array")
    return not packages and _runtime(before_manifest_bytes, "public-before") is None


def activates_first_runtime(
    *,
    transition: TransitionKind,
    before_manifest_bytes: bytes,
) -> bool:
    """Whether this pairing activates a runtime onto a channel that had none.

    True for a first release, and also for a channel that published packages
    before it published a runtime. Both cases install the candidate directly
    rather than upgrading onto it, so neither has a predecessor to boot first.
    """
    if transition is TransitionKind.FRESH_INSTALL:
        return True
    return _runtime(before_manifest_bytes, "public-before") is None and transition in {
        TransitionKind.RUNTIME_ONLY,
        TransitionKind.RUNTIME_THEN_BINARY,
    }


def resolve_public_before_package(
    *,
    supplied: str | Path | None,
    before_manifest_bytes: bytes,
) -> tuple[Path | None, ArtifactIdentity | None]:
    """Resolve the predecessor a pairing upgrades from, which a first release lacks.

    Required in both directions. A published graph without its package would
    silently become a fresh-install proof, and a first release carrying one
    would claim a predecessor nobody could have installed.
    """
    if public_before_is_unpublished(before_manifest_bytes):
        if supplied is not None:
            raise SystemExit(
                "exact pairing supplied a public-before package for a channel that has "
                "published none"
            )
        return None, None
    if supplied is None:
        raise SystemExit("exact pairing requires the public-before package the channel is serving")
    package = Path(supplied)
    return package, artifact_identity_from_manifest_package(before_manifest_bytes, package)


def verify_candidate_runtime_publication(
    *,
    after_manifest: Path,
    publication_base: object,
    release_dir: object,
) -> None:
    """Prove a staged candidate publication is the one its manifest selects.

    Every pairing that stages a runtime asks this identical question through
    this one subprocess call, so a fix cannot leave a copy behind.
    """
    command = [
        sys.executable,
        str(ROOT / "build_system" / "scripts" / "release" / "verify-runtime-publication.py"),
        "--manifest",
        str(after_manifest),
        "--publication-base",
        str(publication_base),
        "--release-dir",
        str(release_dir),
    ]
    print("+ " + " ".join(command), flush=True)
    try:
        subprocess.run(command, cwd=ROOT, check=True)
    except subprocess.CalledProcessError as error:
        raise SystemExit(
            "exact pairing candidate runtime publication failed verification"
        ) from error


def classify_pairing_inputs(
    *,
    channel: str,
    baseline_channel: str | None = None,
    before_manifest_bytes: bytes,
    after_manifest_bytes: bytes,
    before_artifact: ArtifactIdentity | None,
    after_artifact: ArtifactIdentity,
) -> TransitionKind:
    """Classify an exact release pairing by what it changes."""

    first_release = public_before_is_unpublished(before_manifest_bytes)
    baseline = baseline_channel or channel

    if first_release and baseline != channel:
        raise GlowupContractError(f"{channel} release has no published {baseline} baseline channel")
    if first_release:
        if _runtime(after_manifest_bytes, "candidate-after") is None:
            raise GlowupContractError("a first release must publish a runtime")
        transition_kind = TransitionKind.FRESH_INSTALL
    elif baseline != channel:
        transition_kind = TransitionKind.CHANNEL_SWITCH
    elif _runtime(before_manifest_bytes, "public-before") == _runtime(
        after_manifest_bytes, "candidate-after"
    ):
        transition_kind = TransitionKind.BINARY_ONLY
    elif (
        before_artifact is not None
        and before_artifact.version == after_artifact.version
        and before_artifact.sha256 == after_artifact.sha256
    ):
        transition_kind = TransitionKind.RUNTIME_ONLY
    else:
        transition_kind = TransitionKind.RUNTIME_THEN_BINARY

    validate_pairing_inputs(
        kind=transition_kind,
        channel=channel,
        baseline_channel=baseline,
        before_manifest_bytes=before_manifest_bytes,
        after_manifest_bytes=after_manifest_bytes,
        before_artifact=before_artifact,
        after_artifact=after_artifact,
    )
    return transition_kind

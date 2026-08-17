#!/usr/bin/env python3
"""Whether a release pairing is a channel's first public release, and its kind.

A channel that has never served a working binary graph has no predecessor to
upgrade from. The lane already reaches that conclusion by itself: a retired
public graph resolves to `bootstrap`, and the projected public-before manifest
then declares no packages and no profiles. What was missing is that the
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
from collections.abc import Mapping
from pathlib import Path
from typing import cast

try:
    from release_glowup import (
        ArtifactIdentity,
        GlowupContractError,
        TransitionKind,
        artifact_identity_from_manifest_package,
        load_manifest_bytes,
        validate_pairing_inputs,
    )
except ModuleNotFoundError:
    from scripts.release_glowup import (
        ArtifactIdentity,
        GlowupContractError,
        TransitionKind,
        artifact_identity_from_manifest_package,
        load_manifest_bytes,
        validate_pairing_inputs,
    )

ROOT = Path(__file__).resolve().parents[1]


def _profile_map(manifest_bytes: bytes, label: str) -> Mapping[str, object]:
    manifest = load_manifest_bytes(manifest_bytes)
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict):
        raise GlowupContractError(f"{label} manifest profiles must be an object")
    if not all(isinstance(profile_id, str) for profile_id in profiles):
        raise GlowupContractError(f"{label} manifest profile ids must be strings")
    return cast(Mapping[str, object], profiles)


def public_before_is_unpublished(before_manifest_bytes: bytes) -> bool:
    """Whether the public-before graph offers nothing a user could have installed.

    Both halves have to be empty. A graph with profiles but no packages is not a
    first release -- it is a broken one, and calling it fresh would skip the
    upgrade proof for a channel that really does have a predecessor.
    """
    manifest = load_manifest_bytes(before_manifest_bytes)
    packages = manifest.get("packages")
    if not isinstance(packages, list):
        raise GlowupContractError("public-before manifest packages must be an array")
    return not packages and not _profile_map(before_manifest_bytes, "public-before")


def activates_first_profiles(
    *,
    transition: TransitionKind,
    before_manifest_bytes: bytes,
) -> bool:
    """Whether this pairing activates a profile set onto a channel that had none.

    True for a first release, and also for a channel that published packages
    before it published any profile. Both cases install the candidate directly
    rather than upgrading onto it, so neither has a predecessor to boot first.
    """
    if transition is TransitionKind.FRESH_INSTALL:
        return True
    return not _profile_map(before_manifest_bytes, "public-before") and transition in {
        TransitionKind.PROFILE_ONLY,
        TransitionKind.PROFILE_THEN_BINARY,
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


def verify_candidate_profile_publication(
    *,
    after_manifest: Path,
    profile: object,
    publication_base: object,
    release_dir: object,
) -> None:
    """Prove a staged candidate publication is the one its manifest selects.

    The profile-only and profile-then-binary pairings ask this identical
    question, and used to ask it through two identical copies of the same
    subprocess call -- so a fix to one would have left the other behind.
    """
    command = [
        sys.executable,
        str(ROOT / "scripts" / "verify-profile-publication.py"),
        "--manifest",
        str(after_manifest),
        "--profile",
        str(profile),
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
            "exact pairing candidate profile publication failed verification"
        ) from error


def classify_pairing_inputs(
    *,
    channel: str,
    before_manifest_bytes: bytes,
    after_manifest_bytes: bytes,
    before_artifact: ArtifactIdentity | None,
    after_artifact: ArtifactIdentity,
) -> tuple[TransitionKind, tuple[str, ...]]:
    """Classify a binary lane and return its complete staged profile set."""

    before_profile_map = _profile_map(before_manifest_bytes, "public-before")
    after_profile_map = _profile_map(after_manifest_bytes, "candidate-after")
    first_release = public_before_is_unpublished(before_manifest_bytes)

    if first_release:
        transition_kind = TransitionKind.FRESH_INSTALL
        # Every profile the candidate declares is staged, because none of them
        # were ever served: there is no unchanged remainder to leave alone.
        changed = sorted(after_profile_map)
        if not changed:
            raise GlowupContractError("a first release must publish at least one profile")
    else:
        changed = sorted(
            profile_id
            for profile_id in set(before_profile_map) | set(after_profile_map)
            if before_profile_map.get(profile_id) != after_profile_map.get(profile_id)
        )
        transition_kind = (
            TransitionKind.BINARY_ONLY if not changed else TransitionKind.PROFILE_THEN_BINARY
        )

    validate_pairing_inputs(
        kind=transition_kind,
        channel=channel,
        before_manifest_bytes=before_manifest_bytes,
        after_manifest_bytes=after_manifest_bytes,
        before_artifact=before_artifact,
        after_artifact=after_artifact,
        changed_profiles=changed,
    )
    return transition_kind, tuple(changed)

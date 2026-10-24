"""Finite cross-step receipts for the existing native handoff, not an uploader."""

from __future__ import annotations

import os
import re
import tempfile
import time
from pathlib import Path
from typing import Annotated, Literal
from uuid import UUID

import psutil
from capsem_builder.cache.leases import active_path, lock_holders
from capsem_builder.gate.configschema import Strict
from pydantic import Field, StringConstraints, model_validator

from . import native_candidates as candidates
from . import producer_evidence
from .native_artifact import Bundle
from .native_inputs import BaseBinding

SECTIONS = (
    "debug_image",
    "reference_image",
    "codex-cli",
    "claude-code",
    "agy",
    "antigravity-desktop",
)
ArtifactId = Annotated[str, StringConstraints(pattern=r"^[1-9][0-9]{0,19}$")]


class Owner(Strict):
    key: str
    source: candidates.Source
    section: candidates.Section | Literal["base"]
    pid: int = Field(gt=0)
    created: float = Field(gt=0, allow_inf_nan=False)
    nonce: UUID
    expires: float = Field(gt=0, allow_inf_nan=False)
    device: int = Field(ge=0)
    inode: int = Field(gt=0)
    bundle: Bundle | None = None
    base: BaseBinding | None = None


class Request(Strict):
    owner: Owner
    action: Literal["release", "abort"]
    artifact_id: ArtifactId | None = None

    @model_validator(mode="after")
    def exact_completion(self):
        needs_id = self.action == "release" and self.owner.section != "base"
        if needs_id != (self.artifact_id is not None):
            raise ValueError("only verified artifact release requires an artifact ID")
        if self.artifact_id is not None and int(self.artifact_id) >= 2**64:
            raise ValueError("artifact ID exceeds unsigned 64-bit range")
        return self


class Completion(Strict):
    request: Request
    finished: float = Field(gt=0, allow_inf_nan=False)


def key(config, section, run, attempt):
    if section not in (*SECTIONS, "base") or any(
        not re.fullmatch(r"[1-9][0-9]{0,19}", value) for value in (run, attempt)
    ):
        raise ValueError("native transfer requires fixed section and positive run/attempt")
    return f"{config.functional.native_images.transfer.prefix}-{candidates.source(config.root).commit}-{run}-{attempt}-{section}"


def control(config, name, phase):
    if not re.fullmatch(
        r"native-transfer-[0-9a-f]{40}-[1-9][0-9]{0,19}-[1-9][0-9]{0,19}-(?:"
        + "|".join((*SECTIONS, "base"))
        + r")",
        name,
    ):
        raise ValueError("native owner key differs from fixed identity")
    if phase not in ("ready", "request", "complete"):
        raise ValueError("unknown native owner control phase")
    root = config.path(config.functional.native_images.receipt_dir)
    return candidates.contained(config.root, root / f"{name}-{phase}.json")


def publish(path, value, maximum):
    data = value.model_dump_json().encode()
    if len(data) > maximum:
        raise ValueError("native owner control exceeds metadata budget")
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(dir=path.parent, prefix=".native-owner-")
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
        # No replacement: stale invocation, duplicate signal and symlink all fail.
        os.link(temporary, path, follow_symlinks=False)
    finally:
        Path(temporary).unlink()


def read(config, path, model):
    return model.model_validate_json(
        producer_evidence.read_file(path, config.functional.native_images.receipt_max_bytes)
    )


def lease(config, paths, owner):
    settings = config.functional.native_images
    if owner.section == "base":
        if owner.base is None:
            raise ValueError("base owner has no original base binding")
        stage = settings.base.cache_stage
        generation = "native-base-amd64-" + owner.base.input_key
    else:
        stage, generation = settings.transfer.stage, owner.key
    template = paths.policy.stages[stage].lease_template
    if template is None:
        raise ValueError("native owner stage lacks lease")
    return paths.stage(stage) / template.format(key=generation)


def require_live(config, paths, owner):
    if owner.source != candidates.source(config.root) or time.time() >= owner.expires:
        raise ValueError("native owner source or deadline differs")
    control(config, owner.key, "ready")
    try:
        process = psutil.Process(owner.pid)
        if (
            process.create_time() != owner.created
            or not process.is_running()
            or process.status() == psutil.STATUS_ZOMBIE
        ):
            raise ValueError("native owner process generation differs")
        path = lease(config, paths, owner)
        info = path.lstat()
        if (
            path.is_symlink()
            or (info.st_dev, info.st_ino) != (owner.device, owner.inode)
            or not active_path(path)
            or not any(holder.startswith(f"pid {owner.pid} (") for holder in lock_holders(path))
        ):
            raise ValueError("native owner lease inode or kernel holder differs")
    except (OSError, psutil.Error) as error:
        raise ValueError("native owner no longer holds its lease") from error


def accept(owner, request):
    if request.owner != owner:
        raise ValueError("native control targets stale or foreign owner")
    return request.artifact_id


def complete_sections(rows):
    sections = [row["section"] for row in rows]
    ids = [row["artifact_id"] for row in rows]
    if (
        len(rows) != len(SECTIONS)
        or set(sections) != set(SECTIONS)
        or len(set(ids)) != len(SECTIONS)
    ):
        raise ValueError("native manifest requires six unique completed artifacts")
    return sorted(rows, key=lambda row: SECTIONS.index(row["section"]))

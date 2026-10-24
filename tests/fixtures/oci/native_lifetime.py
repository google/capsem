"""Existing handoff CLI's finite lease owner across official artifact actions."""

from __future__ import annotations

import os
import time
from contextlib import contextmanager
from uuid import uuid4

import psutil
from capsem_builder.cache.config import load_paths
from capsem_builder.cache.leases import active_path, release_path, retain_generation

from . import native_artifact, native_transfer
from . import native_candidates as candidates
from . import native_owner as protocol
from .native_build import candidate
from .native_inputs import BaseBinding, workflow_key
from .pinned_image import SECTIONS


def image(config, section):
    return SECTIONS.get(section) or candidate(config, section)


def root(config, paths, owner):
    return candidates.contained(
        paths.stage(config.functional.native_images.transfer.stage),
        paths.stage(config.functional.native_images.transfer.stage) / owner.key,
    )


def _serve(config, paths, owner):
    settings = config.functional.native_images
    maximum = settings.receipt_max_bytes
    protocol.require_live(config, paths, owner)
    protocol.publish(protocol.control(config, owner.key, "ready"), owner, maximum)
    duration = (
        settings.transfer.base_hold_seconds
        if owner.section == "base"
        else settings.transfer.hold_seconds
    )
    deadline = time.monotonic() + duration
    request_path = protocol.control(config, owner.key, "request")
    while time.monotonic() < deadline:
        if request_path.exists() or request_path.is_symlink():
            request = protocol.read(config, request_path, protocol.Request)
            protocol.accept(owner, request)
            protocol.require_live(config, paths, owner)
            if request.action == "release" and owner.section != "base":
                native_artifact.verify(
                    config, root(config, paths, owner) / "download", expected=owner.bundle
                )
                native_transfer.capacity(paths, settings.transfer.stage)
            return request
        time.sleep(settings.transfer.poll_seconds)
    raise TimeoutError("native artifact owner expired without verified consumer completion")


@contextmanager
def base(config, paths):
    settings = config.functional.native_images
    selected = candidates.Receipt.model_validate_json(
        protocol.producer_evidence.read_file(
            candidates.receipt_path(config, "reference_image"), settings.receipt_max_bytes
        )
    )
    if selected.source != candidates.source(config.root) or selected.build is None:
        raise ValueError("base retention requires original current development build")
    binding = selected.build.base
    if binding.input_key != workflow_key(config.root, "base"):
        raise ValueError("base producer input key differs")
    generation = "native-base-amd64-" + binding.input_key
    stage = settings.base.cache_stage
    template = paths.policy.stages[stage].lease_template
    if template is None:
        raise ValueError("base stage has no original lease")
    retain_generation(paths, stage, generation)
    try:
        layout = paths.stage(stage) / generation / "layout"
        candidates.contained(paths.stage(stage), layout)
        actual = BaseBinding.capture(
            layout,
            binding.subject,
            settings.platform,
            binding.input_key,
            maximum=settings.receipt_max_bytes,
        )
        if actual != binding:
            raise ValueError("retained base differs from original development prefix")
        native_transfer.capacity(paths, stage)
        yield binding
    finally:
        release_path(paths.stage(stage) / template.format(key=generation))


def hold(config, section, run, attempt):
    settings = config.functional.native_images
    if candidates.host_platform() != settings.platform:
        raise ValueError("native owner requires the actual configured Linux amd64 host")
    paths = load_paths(config.root)
    key = protocol.key(config, section, run, attempt)
    context = (
        base(config, paths)
        if section == "base"
        else native_artifact.prepare(config, paths, image(config, section), key)
    )
    with context as retained:
        bundle = None if section == "base" else retained[1]
        binding = retained if section == "base" else None
        owner = protocol.Owner(
            key=key,
            source=candidates.source(config.root),
            section=section,
            pid=os.getpid(),
            created=psutil.Process().create_time(),
            nonce=uuid4(),
            expires=time.time()
            + (
                settings.transfer.base_hold_seconds
                if section == "base"
                else settings.transfer.hold_seconds
            ),
            device=0,
            inode=1,
            bundle=bundle,
            base=binding,
        )
        info = protocol.lease(config, paths, owner).lstat()
        owner = owner.model_copy(update={"device": info.st_dev, "inode": info.st_ino})
        request = _serve(config, paths, owner)
    # A completion is published only AFTER both original and destination leases close.
    protocol.publish(
        protocol.control(config, key, "complete"),
        protocol.Completion(request=request, finished=time.time()),
        settings.receipt_max_bytes,
    )


def ready(config, section, run, attempt):
    settings = config.functional.native_images
    paths = load_paths(config.root)
    key = protocol.key(config, section, run, attempt)
    path = protocol.control(config, key, "ready")
    deadline = time.monotonic() + settings.transfer.ready_seconds
    while time.monotonic() < deadline:
        if path.exists() or path.is_symlink():
            owner = protocol.read(config, path, protocol.Owner)
            if owner.key != key or owner.section != section:
                raise ValueError("native ready owner differs from requested invocation")
            protocol.require_live(config, paths, owner)
            if section != "base":
                native_artifact.verify(
                    config, root(config, paths, owner) / "upload", expected=owner.bundle
                )
                native_transfer.capacity(paths, settings.transfer.stage)
            return owner
        time.sleep(settings.transfer.poll_seconds)
    raise TimeoutError("native handoff owner did not become ready")


def settle(config, section, run, attempt, *, abort=False, artifact_id=None):
    settings = config.functional.native_images
    paths = load_paths(config.root)
    key = protocol.key(config, section, run, attempt)
    complete = protocol.control(config, key, "complete")
    owner = protocol.read(config, protocol.control(config, key, "ready"), protocol.Owner)
    protocol.require_live(config, paths, owner)
    if owner.key != key or owner.section != section:
        raise ValueError("native settlement invocation differs")
    if not abort and section != "base":
        native_artifact.verify(
            config, root(config, paths, owner) / "download", expected=owner.bundle
        )
        native_transfer.capacity(paths, settings.transfer.stage)
    request = protocol.Request(
        owner=owner, action="abort" if abort else "release", artifact_id=artifact_id
    )
    request_path = protocol.control(config, key, "request")
    if request_path.exists():
        existing = protocol.read(config, request_path, protocol.Request)
        protocol.accept(owner, existing)
        if not abort and existing != request:
            raise ValueError("native settlement cannot replace a different request")
        request = existing
    else:
        protocol.publish(request_path, request, settings.receipt_max_bytes)
    deadline = time.monotonic() + settings.transfer.release_seconds
    while time.monotonic() < deadline:
        if complete.exists() or complete.is_symlink():
            result = protocol.read(config, complete, protocol.Completion)
            if (
                result.request != request
                or result.finished > owner.expires
                or active_path(protocol.lease(config, paths, owner))
            ):
                raise ValueError("native completion differs or original lease is still held")
            return result
        time.sleep(settings.transfer.poll_seconds)
    raise TimeoutError("native owner did not release after consumer completion")


def finish(config, run, attempt):
    from .native_manifest import finish as completed_manifest

    return completed_manifest(config, run, attempt)

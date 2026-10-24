"""Typed action outputs for the existing native handoff CLI only."""

from __future__ import annotations

import os

from capsem_builder.cache.config import load_paths
from capsem_builder.gate.configschema import Strict
from pydantic import Field

from . import native_candidates as identity
from . import native_lifetime as lifetime
from . import native_owner as protocol


class Qualified(Strict):
    section: identity.Section
    artifact_id: protocol.ArtifactId
    receipt: identity.Receipt


class Manifest(Strict):
    source: identity.Source
    platform: str
    candidates: list[Qualified] = Field(min_length=6, max_length=6)


def outputs(values):
    with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
        for name, value in values.items():
            if any(character in str(value) for character in "\r\n"):
                raise ValueError("native action output is not a single line")
            stream.write(f"{name}={value}\n")


def command(config, args):
    settings = config.functional.native_images
    transfer = settings.transfer
    if args.action == "configure":
        if any((args.section, args.run_id, args.attempt, args.artifact_id)):
            raise ValueError("configuration accepts no candidate or run controls")
        outputs(
            {
                "outer_seconds": transfer.outer_seconds,
                "base_outer_seconds": transfer.base_outer_seconds,
                "ready_seconds": transfer.ready_seconds,
                "settle_seconds": transfer.verify_seconds + transfer.release_seconds,
                "upload_minutes": transfer.upload_minutes,
                "download_minutes": transfer.download_minutes,
                "control_dir": config.path(settings.receipt_dir),
            }
        )
        return
    if not args.run_id or not args.attempt or (args.action != "release" and args.artifact_id):
        raise ValueError("native owner requires run/attempt and action-bound artifact ID")
    if args.action == "finish":
        if args.section:
            raise ValueError("finish requires all six candidates, not one section")
        manifest = Manifest.model_validate(lifetime.finish(config, args.run_id, args.attempt))
        path = identity.contained(config.root, config.path(settings.artifact_dir) / "handoff.json")
        protocol.publish(path, manifest, settings.receipt_max_bytes)
        outputs({"manifest_path": path})
        return
    if args.section not in (*protocol.SECTIONS, "base"):
        raise ValueError("native owner requires a fixed section")
    if args.action == "hold":
        lifetime.hold(config, args.section, args.run_id, args.attempt)
    elif args.action == "ready":
        owner = lifetime.ready(config, args.section, args.run_id, args.attempt)
        if args.section != "base":
            root = lifetime.root(config, load_paths(config.root), owner)
            outputs(
                {
                    "artifact_name": owner.key,
                    "artifact_path": root / "upload",
                    "download_path": root / "download",
                }
            )
    elif args.action in ("release", "abort"):
        lifetime.settle(
            config,
            args.section,
            args.run_id,
            args.attempt,
            abort=args.action == "abort",
            artifact_id=args.artifact_id,
        )
    else:
        raise ValueError("unknown native handoff action")

"""One exact qualified native artifact, copied and checked by existing owners."""

from __future__ import annotations

import json
from contextlib import contextmanager
from pathlib import Path

from capsem_builder.gate.configschema import Strict
from pydantic import field_validator

from . import native_candidates as candidates
from . import native_handoff, native_proof, native_transfer, producer_evidence


class Bundle(Strict):
    receipt: candidates.Receipt
    source_root: Path
    original_layout: Path

    @field_validator("source_root", "original_layout")
    @classmethod
    def absolute_original_invocation(cls, value):
        if not value.is_absolute() or ".." in value.parts or any(c in str(value) for c in "\r\n"):
            raise ValueError("native original invocation path is not absolute and contained")
        return value


def specification(config, section):
    if section in ("debug_image", "reference_image"):
        return getattr(config.functional, section)
    return config.functional.native_images.apps[section]


def verify(config, directory, *, expected=None):
    settings = config.functional.native_images
    actual = set()
    for path in producer_evidence.regular_files(directory, maximum=4 * settings.artifact_max_files):
        actual.add(path.relative_to(directory))
        if len(actual) > settings.artifact_max_files:
            raise ValueError("native artifact file budget exceeded")
    bundle = Bundle.model_validate_json(
        producer_evidence.read_file(directory / "handoff.json", settings.receipt_max_bytes)
    )
    receipt = bundle.receipt
    spec = specification(config, receipt.section)
    if (
        (expected is not None and bundle != expected)
        or receipt.source != candidates.source(config.root)
        or receipt.platform != settings.platform
        or receipt.image != spec.name
        or receipt.layout_key != receipt.image + "-" + receipt.subject[7:]
        or bundle.original_layout.name != receipt.layout_key
    ):
        raise ValueError("native artifact source/platform/subject differs")
    if receipt.section != "debug_image" and receipt.build is None:
        raise ValueError("official native artifact requires current shared-base binding")
    layout = directory / "layout"
    evidence = producer_evidence.ready(
        layout, spec.repository, receipt.platform.split("/")[1], maximum=spec.evidence_max_bytes
    )
    original = json.loads(
        producer_evidence.read_file(evidence / "receipt.json", spec.evidence_max_bytes)
    )
    manifest = json.loads(
        producer_evidence.read_file(
            layout / "blobs/sha256" / receipt.subject[7:], settings.receipt_max_bytes
        )
    )
    if (
        original["subject"]["digest"] != receipt.subject
        or manifest["config"]["digest"] != receipt.config
        or original["root"]["digest"] != receipt.root
        or producer_evidence.read_file(evidence / "rootfs.blake3", spec.evidence_max_bytes)
        .decode()
        .strip()
        != receipt.root_integrity
    ):
        raise ValueError("native receipt differs from actual original descriptors")
    copied = candidates.Receipt.model_validate_json(
        producer_evidence.read_file(directory / "build.json", settings.receipt_max_bytes)
    )
    if copied != receipt:
        raise ValueError("native original build receipt differs")
    if receipt.build is not None:
        receipt.build.require(
            config.root,
            receipt.image,
            layout,
            receipt.subject,
            receipt.platform,
            settings.receipt_max_bytes,
        )
    # Original paths determine only expected journal argv. They are never opened.
    source_view = config.model_copy(update={"root": bundle.source_root})
    outcome = native_proof.report(source_view, receipt, layout)
    native_proof.match(
        source_view, receipt, bundle.original_layout, outcome, directory / "qualification"
    )
    native_proof.built(source_view, receipt, directory=directory / "producer")
    members = {
        Path("layout") / path
        for path in native_handoff.files(
            layout,
            settings.report_name,
            maximum=settings.artifact_max_files,
            metadata_max=settings.receipt_max_bytes,
        )
    }
    members |= {
        Path("handoff.json"),
        Path("build.json"),
        Path("qualification") / config.runlog.events,
        Path("producer") / config.runlog.events,
    }
    if actual != members or len(actual) > settings.artifact_max_files:
        raise ValueError("native artifact closure or file budget differs")
    if sum((directory / path).stat().st_size for path in actual) > settings.artifact_max_bytes:
        raise ValueError("native artifact byte budget exceeded")
    return bundle


@contextmanager
def prepare(config, paths, image, key):
    settings = config.functional.native_images
    if candidates.host_platform() != settings.platform:
        raise ValueError("native artifact production requires actual Linux amd64")
    source = candidates.source(config.root)
    with native_handoff.leased(config, image) as (receipt, layout):
        native_transfer.capacity(paths, image.settings().cache_stage)
        outcome = native_proof.report(config, receipt, layout)
        journal = native_proof.completed(config, receipt, layout, outcome)
        producer = native_proof.built(config, receipt)
        bundle = Bundle(receipt=receipt, source_root=config.root, original_layout=layout)
        encoded = bundle.model_dump_json().encode()
        if len(encoded) > settings.receipt_max_bytes:
            raise ValueError("native artifact metadata budget exceeded")
        members = native_handoff.files(
            layout,
            settings.report_name,
            maximum=settings.artifact_max_files,
            metadata_max=settings.receipt_max_bytes,
        )
        extras = [
            (candidates.receipt_path(config, image.section), Path("build.json")),
            (journal, Path("qualification") / config.runlog.events),
            (producer, Path("producer") / config.runlog.events),
        ]
        copies = [(layout / path, Path("layout") / path) for path in members] + extras
        total = sum(path.stat().st_size for path, _ in copies) + len(encoded)
        if total > settings.artifact_max_bytes or len(copies) + 1 > settings.artifact_max_files:
            raise ValueError("native artifact byte/file budget exceeded")
        # Upload and independent download coexist under this one retained owner.
        with native_transfer.destination(paths, key, 2 * total) as root:
            upload = root / "upload"
            upload.mkdir()
            remaining = settings.artifact_max_bytes - len(encoded)
            for original, relative in copies:
                target = upload / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                remaining = native_handoff.copy_original(original, target, remaining)
            (upload / "handoff.json").write_bytes(encoded)
            verify(config, upload, expected=bundle)
            native_transfer.capacity(paths, settings.transfer.stage, total)
            if candidates.source(config.root) != source:
                raise ValueError("source changed during native artifact copy")
            yield root, bundle

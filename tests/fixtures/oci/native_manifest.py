"""Completion of six independently downloaded, verified official-action artifacts."""

from capsem_builder.cache.config import load_paths
from capsem_builder.cache.leases import active_path

from . import native_candidates as candidates
from . import native_owner as protocol
from .native_artifact import specification
from .native_inputs import workflow_key


def finish(config, run, attempt):
    settings = config.functional.native_images
    source = candidates.source(config.root)
    paths = load_paths(config.root)
    rows, bases = [], []
    for section in protocol.SECTIONS:
        key = protocol.key(config, section, run, attempt)
        result = protocol.read(
            config, protocol.control(config, key, "complete"), protocol.Completion
        )
        request, owner = result.request, result.request.owner
        bundle = owner.bundle
        if (
            owner.key != key
            or owner.source != source
            or owner.section != section
            or request.action != "release"
            or result.finished > owner.expires
            or bundle is None
            or bundle.receipt.section != section
            or bundle.receipt.source != source
            or bundle.receipt.platform != settings.platform
            or bundle.receipt.image != specification(config, section).name
            or active_path(protocol.lease(config, paths, owner))
        ):
            raise ValueError("native manifest has incomplete/foreign/aborted consumer")
        if section != "debug_image":
            binding = bundle.receipt.build
            if (
                binding is None
                or binding.base.input_key != workflow_key(config.root, "base")
                or binding.input_key
                != workflow_key(config.root, bundle.receipt.image, binding.base.subject)
            ):
                raise ValueError("official artifact lacks current original shared-base inputs")
            binding.base.decoded(settings.platform)
            bases.append(binding.base)
        rows.append(
            {
                "section": section,
                "artifact_id": request.artifact_id,
                "receipt": bundle.receipt.model_dump(mode="json"),
            }
        )
    if len(bases) != 5 or any(binding != bases[0] for binding in bases):
        raise ValueError("all five official artifacts require the SAME original base")
    base_key = protocol.key(config, "base", run, attempt)
    completion = protocol.read(
        config, protocol.control(config, base_key, "complete"), protocol.Completion
    )
    owner = completion.request.owner
    if (
        completion.request.action != "release"
        or owner.source != source
        or owner.section != "base"
        or owner.base != bases[0]
        or owner.key != base_key
        or completion.finished > owner.expires
        or active_path(protocol.lease(config, paths, owner))
    ):
        raise ValueError("shared base lifetime was not completed by its original owner")
    return {
        "source": source.model_dump(),
        "platform": settings.platform,
        "candidates": protocol.complete_sections(rows),
    }

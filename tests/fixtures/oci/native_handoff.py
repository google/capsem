"""Select exact receipts and stage only fully qualified native CI candidates."""

from __future__ import annotations

import argparse
import json
import os
import re
import stat
import tempfile
from contextlib import ExitStack, contextmanager
from pathlib import Path

from capsem_builder.cache.config import load_paths
from capsem_builder.cache.leases import release_path, retain_generation
from capsem_builder.gate.config import load

from tests.fixtures.oci import native_candidates as candidates
from tests.fixtures.oci import native_proof, producer_evidence
from tests.fixtures.oci.pinned_image import CHUNK, ROOT, SECTIONS


@contextmanager
def leased(config, image):
    path = candidates.receipt_path(config, image.section)
    receipt = candidates.Receipt.model_validate_json(
        producer_evidence.read_file(path, config.functional.native_images.receipt_max_bytes)
    )
    paths = load_paths(config.root)
    stage = image.settings().cache_stage
    if paths.stage(stage) != image.stage():
        raise ValueError("candidate lease stage differs")
    template = paths.policy.stages[stage].lease_template
    if template is None:
        raise ValueError("candidate stage has no lease control")
    retain_generation(paths, stage, receipt.layout_key)
    try:
        yield candidates.selected(config, image)
    finally:
        release_path(paths.stage(stage) / template.format(key=receipt.layout_key))


def files(layout, report_name):
    """Exact descriptor closure, never upload an arbitrary cache tree."""
    producer_evidence.safe_tree(layout)
    result = {Path("index.json"), Path(report_name)}
    if (layout / "oci-layout").exists():
        result.add(Path("oci-layout"))
    for relative in (Path(), Path("rootfs-v2")):
        root = layout / relative
        result.add(relative / "index.json")
        if (root / "oci-layout").exists():
            result.add(relative / "oci-layout")
        (entry,) = json.loads((root / "index.json").read_bytes())["manifests"]
        manifest_path = relative / "blobs/sha256" / entry["digest"].split(":")[1]
        result.add(manifest_path)
        manifest = json.loads((layout / manifest_path).read_bytes())
        for descriptor in (manifest["config"], *manifest["layers"]):
            if not re.fullmatch(r"sha256:[0-9a-f]{64}", descriptor["digest"]):
                raise ValueError("invalid artifact descriptor")
            result.add(relative / "blobs/sha256" / descriptor["digest"].split(":")[1])
    result.update(
        Path(producer_evidence.DIRECTORY) / name
        for name in (*producer_evidence.FILES, "receipt.json")
    )
    actual = {path.relative_to(layout) for path in layout.rglob("*") if path.is_file()}
    if actual != result:
        raise ValueError("candidate artifact membership differs from exact allowlist")
    return sorted(result)


def copy_original(source, destination, remaining):
    """Bound the actual drain, not only a pre-copy stat; preserve original bytes."""
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or not 0 < info.st_size <= remaining:
            raise ValueError("native artifact copy type/byte budget exceeded")
        written = 0
        with destination.open("xb") as output:
            while chunk := stream.read(min(CHUNK, info.st_size - written + 1)):
                written += len(chunk)
                if written > info.st_size:
                    raise ValueError("native artifact source grew during copy")
                output.write(chunk)
        if written != info.st_size:
            raise ValueError("native artifact source truncated during copy")
    return remaining - written


def stage(config):
    settings = config.functional.native_images
    if candidates.host_platform() != settings.platform:
        raise ValueError("native artifact staging requires Linux amd64")
    destination = candidates.contained(config.root, config.path(settings.artifact_dir))
    if destination.exists():
        raise ValueError("native artifact destination already exists")
    source = candidates.source(config.root)
    with ExitStack() as stack:
        inputs = []
        for image in SECTIONS.values():
            receipt, layout = stack.enter_context(leased(config, image))
            outcome = native_proof.report(config, receipt, layout)
            journal = native_proof.completed(config, receipt, layout, outcome)
            build_journal = native_proof.built(config, receipt)
            inputs.append(
                (
                    receipt.section,
                    receipt,
                    layout,
                    journal,
                    build_journal,
                    files(layout, settings.report_name),
                )
            )
        total = sum(
            (layout / name).stat().st_size
            for _, _, layout, _, _, members in inputs
            for name in members
        )
        total += sum(
            journal.stat().st_size
            + build_journal.stat().st_size
            + candidates.receipt_path(config, section).stat().st_size
            for section, _, _, journal, build_journal, _ in inputs
        )
        handoff = json.dumps(
            {
                "source": source.model_dump(),
                "platform": settings.platform,
                "candidates": [receipt.model_dump() for _, receipt, _, _, _, _ in inputs],
            },
            sort_keys=True,
        ).encode()
        count = sum(len(members) + 3 for _, _, _, _, _, members in inputs) + 1
        if (
            total + len(handoff) > settings.artifact_max_bytes
            or count > settings.artifact_max_files
        ):
            raise ValueError("native artifact byte/file budget exceeded")
        destination.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            dir=destination.parent, prefix=".native-stage-"
        ) as temporary:
            output = Path(temporary) / "qualified"
            output.mkdir()
            remaining = settings.artifact_max_bytes - len(handoff)
            for section, receipt, layout, journal, build_journal, members in inputs:
                target = output / section
                target.mkdir()
                for relative in members:
                    member = target / "layout" / relative
                    member.parent.mkdir(parents=True, exist_ok=True)
                    remaining = copy_original(layout / relative, member, remaining)
                remaining = copy_original(
                    candidates.receipt_path(config, section), target / "build.json", remaining
                )
                remaining = copy_original(journal, target / config.runlog.events, remaining)
                remaining = copy_original(build_journal, target / "build-journal.jsonl", remaining)
                producer_evidence.ready(
                    target / "layout",
                    SECTIONS[section].settings().repository,
                    receipt.platform.split("/")[1],
                    maximum=SECTIONS[section].settings().evidence_max_bytes,
                )
                copied = candidates.Receipt.model_validate_json(
                    (target / "build.json").read_bytes()
                )
                if copied != receipt:
                    raise ValueError("native build receipt changed during copy")
                copied_outcome = native_proof.report(config, receipt, target / "layout")
                native_proof.match(config, receipt, layout, copied_outcome, target)
            if candidates.source(config.root) != source:
                raise ValueError("source changed during native artifact staging")
            (output / "handoff.json").write_bytes(handoff)
            output.rename(destination)
    return destination


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("select", "stage"))
    parser.add_argument(
        "section",
        nargs="?",
        choices=[*SECTIONS, "codex-cli", "claude-code", "agy", "antigravity-desktop"],
    )
    args = parser.parse_args(argv)
    config = load(ROOT)
    if args.action == "stage":
        if args.section:
            parser.error("stage accepts no section")
        stage(config)
    else:
        if not args.section:
            parser.error("select requires a fixed section")
        # Typed single-line GitHub action output, never parse build stdout.
        from tests.fixtures.oci.native_build import candidate

        image = SECTIONS.get(args.section) or candidate(config, args.section)
        with (
            leased(config, image) as (_, layout),
            Path(os.environ["GITHUB_OUTPUT"]).open("a") as stream,
        ):
            stream.write(config.functional.native_images.layout_output + "=" + str(layout) + "\n")


if __name__ == "__main__":
    main()

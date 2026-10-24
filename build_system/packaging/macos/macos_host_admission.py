"""Refusal-only adapter in the existing ordinary one-package Tart owner."""

from __future__ import annotations

import json
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.release.tools.release_admission_refusal import read_json, validate_report
from capsem_builder.release.tools.release_macos_pairing import cohort
from macos_candidate_content import stage_file


def add_arguments(parser) -> None:
    parser.add_argument("--admission-candidate-package", type=Path)
    parser.add_argument("--admission-candidate-manifest-file", type=Path)
    parser.add_argument("--admission-candidate-sbom", type=Path)


def enabled(args) -> bool:
    values = [
        getattr(args, f"admission_candidate_{key}", None)
        for key in ("package", "manifest_file", "sbom")
    ]
    if any(value is not None for value in values) and not all(
        value is not None for value in values
    ):
        raise ValueError("admission candidate package/manifest/SBOM must be supplied together")
    if all(value is not None for value in values):
        if any(
            getattr(args, key, None) is not None
            for key in ("before_package", "before_manifest_file", "before_sbom")
        ):
            raise ValueError("host admission refusal cannot enter binary Apply mode")
        return True
    if any(
        getattr(args, key, None) is None
        for key in ("updated_manifest_file", "tampered_manifest_file", "incompatible_manifest_file")
    ):
        raise ValueError("ordinary Tart proof requires all transition manifests")
    return False


def prepare(args, *, root: Path, share: Path, guest_package: Path) -> dict | None:
    if not enabled(args):
        return None
    previous = cohort(args.package, args.manifest_file, args.sbom, channel=args.channel)
    candidate = cohort(
        args.admission_candidate_package,
        args.admission_candidate_manifest_file,
        args.admission_candidate_sbom,
        channel=args.channel,
    )
    if (
        previous["artifact"]["sha256"] == candidate["artifact"]["sha256"]
        or previous["source_commit"] == candidate["source_commit"]
    ):
        raise ValueError("host refusal requires original distinct package/source identities")
    if any(row["metadata_manifest_url"] != args.manifest_url for row in (previous, candidate)):
        raise ValueError("host refusal package provenance must match the original fixture URL")
    directory = share / "admission-candidate"
    for source, name in (
        (args.admission_candidate_package, args.admission_candidate_package.name),
        (args.admission_candidate_manifest_file, "manifest.json"),
        (args.admission_candidate_sbom, "capsem-sbom.spdx.json"),
    ):
        stage_file(source, directory / name)
    spec = {
        "before": previous,
        "candidate": candidate,
        "source": args.manifest_url,
        "channel": args.channel,
        "package": guest_package.relative_to(share).as_posix(),
        "sbom": (guest_package.parent / "capsem-sbom.spdx.json").relative_to(share).as_posix(),
        "candidate_package": f"admission-candidate/{args.admission_candidate_package.name}",
        "maximum": gate_config.load(root).benchmark.route_load.max_response_bytes,
    }
    (share / "host-admission-inputs.json").write_text(json.dumps(spec, sort_keys=True) + "\n")
    return spec


def finish_host(share: Path, work: Path, spec: dict) -> None:
    for phase, package, manifest, sbom in (
        ("before", spec["package"], "original-manifest.json", spec["sbom"]),
        (
            "candidate",
            spec["candidate_package"],
            "admission-candidate/manifest.json",
            "admission-candidate/capsem-sbom.spdx.json",
        ),
    ):
        observed = cohort(share / package, share / manifest, share / sbom, channel=spec["channel"])
        if observed != spec[phase]:
            raise ValueError("original refusal package/SBOM/manifest bytes changed")
    report = read_json(share / "report.json", spec["maximum"])
    validate_report(report, spec)
    (work / "report.json").write_text(json.dumps(report, sort_keys=True) + "\n")

"""Optional binary-only pairing mode of the existing Tart install owner."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from capsem_builder import crypto
from capsem_builder.gate import config as gate_config
from capsem_builder.release.tools.release_apply_receipt import activation_receipt
from capsem_builder.release.tools.release_macos_apply import validate_native
from capsem_builder.release.tools.release_macos_pairing import validate_pair, validate_public_pair
from macos_candidate_content import stage_file


def add_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--before-package", type=Path)
    parser.add_argument("--before-manifest-file", type=Path)
    parser.add_argument("--before-sbom", type=Path)
    parser.add_argument("--public-predecessor-upgrade", action="store_true")


def argument_parser(**kwargs) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(**kwargs)
    add_arguments(parser)
    return parser


def validate_options(args) -> bool:
    supplied = [
        value is not None
        for value in (args.before_package, args.before_manifest_file, args.before_sbom)
    ]
    if any(supplied) and not all(supplied):
        raise ValueError("before package/manifest/SBOM must be supplied together")
    if getattr(args, "public_predecessor_upgrade", False) and not all(supplied):
        raise ValueError("public predecessor requires complete original before inputs")
    return all(supplied)


def prepare(args, *, root: Path, share: Path, guest_package: Path) -> dict | None:
    values = (args.before_package, args.before_manifest_file, args.before_sbom)
    if not validate_options(args):
        return None
    public = getattr(args, "public_predecessor_upgrade", False)
    pair = (validate_public_pair if public else validate_pair)(
        before=values, after=(args.package, args.manifest_file, args.sbom), channel=args.channel
    )
    if not public and any(
        pair[phase]["metadata_manifest_url"] != args.manifest_url for phase in ("before", "after")
    ):
        raise ValueError(
            "paired packages must own the exact local Tart polling URL; never rewrite package provenance"
        )
    inputs = {
        "before": {
            "package": f"before/{args.before_package.name}",
            "manifest": "before/manifest.json",
            "sbom": "before/capsem-sbom.spdx.json",
        },
        "after": {
            "package": guest_package.relative_to(share).as_posix(),
            "manifest": "original-manifest.json",
            "sbom": (guest_package.parent / "capsem-sbom.spdx.json").relative_to(share).as_posix(),
        },
    }
    if public:
        # Host-bound to the same original bytes already verified by the pair owner.
        inputs["before"]["manifest_blake3"] = crypto.digest_file(args.before_manifest_file)
    for name, source in zip(("package", "manifest", "sbom"), values, strict=True):
        stage_file(source, share / inputs["before"][name])
    stage_file(
        args.before_manifest_file, share / "candidate" / "assets" / args.channel / "manifest.json"
    )
    limits = gate_config.load(root).benchmark.route_load
    pair.update(
        inputs=inputs,
        limits={"timeout": limits.timeout_seconds, "maximum": limits.max_response_bytes},
    )
    (share / "binary-apply-pair.json").write_text(json.dumps(pair, sort_keys=True) + "\n")
    return pair


def validate_report(path: Path, pair: dict, *, source: str) -> dict:
    maximum = pair["limits"]["maximum"]
    with path.open("rb") as stream:
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError("Tart binary Apply report exceeds configured byte budget")
    report = json.loads(raw)
    public = pair["scope"] == "public_predecessor_upgrade"
    schema = "capsem.macos_public_upgrade.v1" if public else "capsem.macos_binary_apply.v1"
    if report.get("schema") != schema or report.get("scope") != pair["scope"]:
        raise ValueError("Tart binary Apply report has wrong scope")
    if report.get("pair") != {key: pair[key] for key in ("channel", "before", "after")}:
        raise ValueError("Tart binary Apply report has foreign package/source/SBOM pairing")
    capabilities = report.get("capabilities", {})
    if any(
        capabilities.get(key) is not False
        for key in ("physical_vz_boot", "full_doctor", "installed_winterfell")
    ):
        raise ValueError("Tart binary Apply cannot credit either package with physical VZ proof")
    if any(capabilities.get(key) is not True for key in ("native_install", "launchd")):
        raise ValueError("Tart binary Apply omitted its native install/launchd proof")
    activation_receipt(
        report["original_request"],
        report["transition"],
        kind="public_predecessor_upgrade" if public else "binary_only",
        source=source,
        digest=pair["after"]["manifest_sha256"],
        source_commit=pair["after"]["source_commit"],
    )
    if report.get("activation") != "verified":
        raise ValueError("Tart binary Apply did not verify native activation")
    validate_native(report["installed"], report["retirement"], pair, source=source)
    if public:
        from capsem_builder.release.tools.release_macos_public_upgrade import (
            require_original_success,
            validate_evidence,
        )

        require_original_success(report["original_request"])
        validate_evidence(report["public_predecessor"], pair, source=source)
    return report


def finish_host(share: Path, work_dir: Path, pair: dict, *, source: str) -> None:
    report = validate_report(share / "report.json", pair, source=source)
    (work_dir / "report.json").write_text(json.dumps(report, sort_keys=True) + "\n")


def run_paired(args, command: list[str], *, runner) -> Path | None:
    if not validate_options(args):
        return None
    paired = list(command)
    position = paired.index("--work-dir") + 1
    work = Path(paired[position]) / (
        "public-upgrade" if getattr(args, "public_predecessor_upgrade", False) else "binary-apply"
    )
    paired[position] = str(work)
    paired.extend(
        [
            "--before-package",
            str(args.before_package),
            "--before-manifest-file",
            str(args.before_manifest_file),
            "--before-sbom",
            str(args.before_sbom),
        ]
    )
    if getattr(args, "public_predecessor_upgrade", False):
        paired.append("--public-predecessor-upgrade")
    runner(paired)
    return work / "report.json"


def attach(
    report_path: Path,
    paired_path: Path | None,
    args,
    *,
    package: Path,
    manifest: Path,
    sbom: Path,
    source: str,
    root: Path,
) -> None:
    if paired_path is None:
        return
    pair = (
        validate_public_pair
        if getattr(args, "public_predecessor_upgrade", False)
        else validate_pair
    )(
        before=(args.before_package, args.before_manifest_file, args.before_sbom),
        after=(package, manifest, sbom),
        channel=args.channel,
    )
    limits = gate_config.load(root).benchmark.route_load
    pair["limits"] = {"timeout": limits.timeout_seconds, "maximum": limits.max_response_bytes}
    paired = validate_report(paired_path, pair, source=source)
    report = json.loads(report_path.read_text())
    key = (
        "public_predecessor_upgrade"
        if pair["scope"] == "public_predecessor_upgrade"
        else "binary_apply"
    )
    report["adapter_evidence"][key] = paired
    report_path.write_text(json.dumps(report, sort_keys=True) + "\n")

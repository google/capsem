"""Scoped binary-Apply evidence inside the existing Tart fixture, stdlib only."""

from __future__ import annotations

import argparse
import json
import re
import time
from pathlib import Path

from .release_apply_receipt import activation_receipt
from .release_glowup import validate_installed_evidence
from .release_macos_pairing import validate_pair, validate_public_pair


def inputs(root: Path, spec: dict, phase: str) -> tuple[Path, Path, Path]:
    paths = []
    for name in ("package", "manifest", "sbom"):
        relative = Path(spec["inputs"][phase][name])
        resolved = (root / relative).resolve()
        if (
            relative.is_absolute()
            or ".." in relative.parts
            or not resolved.is_relative_to(root.resolve())
        ):
            raise ValueError("paired input escaped the owned Tart share")
        paths.append(resolved)
    return paths[0], paths[1], paths[2]


def validate_staged(root: Path, spec: dict) -> None:
    checked = (
        validate_public_pair if spec.get("scope") == "public_predecessor_upgrade" else validate_pair
    )(
        before=inputs(root, spec, "before"),
        after=inputs(root, spec, "after"),
        channel=spec["channel"],
    )
    for field in ("schema", "scope", "channel", "before", "after"):
        if checked[field] != spec[field]:
            raise ValueError(f"staged paired inputs have foreign {field}")


def _bytes(root: Path, name: str, maximum: int) -> bytes:
    with (root / name).open("rb") as stream:
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("paired evidence exceeds configured byte budget")
    return data


def _read(root: Path, name: str, maximum: int) -> dict:
    value = json.loads(_bytes(root, name, maximum))
    if not isinstance(value, dict):
        raise ValueError("paired evidence must be an object")
    return value


def validate_native(installed: dict, retirement: dict, spec: dict, *, source: str) -> None:
    for phase in ("before", "after"):
        value = installed[phase]
        validate_installed_evidence(value)
        expected_metadata = (
            source
            if spec["scope"] == "public_predecessor_upgrade" and phase == "after"
            else spec[phase]["metadata_manifest_url"]
        )
        if (
            value["package_version"] != spec[phase]["artifact"]["version"]
            or value["channel"] != spec["channel"]
            or value["manifest_url"] != source
            or value.get("metadata_manifest_url") != expected_metadata
        ):
            raise ValueError("paired installed evidence does not identify its exact phase")
    if (
        retirement.get("old_service_retired") is not True
        or type(retirement.get("old_service_pid")) is not int
        or type(retirement.get("new_service_pid")) is not int
        or min(retirement["old_service_pid"], retirement["new_service_pid"]) <= 0
        or retirement["old_service_pid"] == retirement["new_service_pid"]
    ):
        raise ValueError("paired Apply did not prove actual old launchd service retirement")


def finish(root: Path, spec: dict, *, source: str, maximum: int) -> dict:
    validate_staged(root, spec)
    request = _read(root, "binary-apply-request.json", maximum)
    marker = int((root / "binary-apply-audit-line").read_text().strip())
    if marker < 0 or request.get("after_line") != marker:
        raise ValueError("paired original request has a foreign audit marker")
    public = spec["scope"] == "public_predecessor_upgrade"
    result = activation_receipt(
        request,
        _read(root, "binary-apply-transition.json", maximum),
        kind="public_predecessor_upgrade" if public else "binary_only",
        source=source,
        digest=spec["after"]["manifest_sha256"],
        source_commit=spec["after"]["source_commit"],
    )
    installed = {}
    for phase in ("before", "after"):
        evidence = _read(root, f"binary-apply-{phase}-installed.json", maximum)
        evidence.update(package_receipt=True, binary_cohort=True)
        version = _bytes(root, f"binary-apply-{phase}-version.txt", maximum).decode("utf-8")
        revision = re.search(r"\(build ([0-9a-f]{7,40})\.", version)
        if revision is None or not spec[phase]["source_commit"].startswith(revision.group(1)):
            raise ValueError("paired installed CLI has foreign source provenance")
        installed[phase] = evidence
    retirement = _read(root, "binary-apply-retirement.json", maximum)
    validate_native(installed, retirement, spec, source=source)
    elapsed = time.monotonic_ns() - request["started_ns"]
    if elapsed < request["elapsed_ns"]:
        raise ValueError("paired activation predates original request")
    result.update(
        schema="capsem.macos_public_upgrade.v1" if public else "capsem.macos_binary_apply.v1",
        scope=spec["scope"],
        pair={key: spec[key] for key in ("channel", "before", "after")},
        installed=installed,
        retirement=retirement,
        activation_verified_elapsed_ns=elapsed,
        capabilities={
            "native_install": True,
            "launchd": True,
            "physical_vz_boot": False,
            "full_doctor": False,
            "installed_winterfell": False,
        },
    )
    if public:
        from .release_macos_public_upgrade import require_original_success, validate_evidence

        require_original_success(request)
        original = _read(root, "public-upgrade-state.json", maximum)
        validate_evidence(original, spec, source=source)
        result["public_predecessor"] = original
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "field", "binaries", "retirement", "finish"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--phase", choices=("before", "after"))
    parser.add_argument("--field")
    parser.add_argument("--source")
    parser.add_argument("--old-pid", type=int)
    parser.add_argument("--new-pid", type=int)
    args = parser.parse_args()
    spec = json.loads((args.root / "binary-apply-pair.json").read_text())
    if args.command == "check":
        validate_staged(args.root, spec)
    elif args.command == "field":
        if args.phase is None:
            value = (
                spec[args.field]
                if args.field in ("scope", "channel")
                else spec["limits"][args.field]
            )
        elif args.field in ("package", "manifest", "sbom"):
            value = str(
                inputs(args.root, spec, args.phase)[
                    ("package", "manifest", "sbom").index(args.field)
                ]
            )
        elif args.field == "version":
            value = spec[args.phase]["artifact"]["version"]
        else:
            value = spec[args.phase][args.field]
        print(value)
    elif args.command == "binaries":
        print("\n".join(spec[args.phase]["binaries"]))
    elif args.command == "retirement":
        (args.root / "binary-apply-retirement.json").write_text(
            json.dumps(
                {
                    "old_service_pid": args.old_pid,
                    "new_service_pid": args.new_pid,
                    "old_service_retired": True,
                }
            )
            + "\n"
        )
    else:
        if args.source is None:
            parser.error("finish requires --source")
        result = finish(args.root, spec, source=args.source, maximum=spec["limits"]["maximum"])
        (args.root / "report.json").write_text(json.dumps(result, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

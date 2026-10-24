"""Read the original public BEFORE hydration receipt; never poll or repair it.

The existing host pair stager binds the expected BLAKE3 to its SHA256-verified
manifest bytes. Clean Tart Python has no BLAKE3 module; it must not recompute or
invent that identity. This is a one-fixture receipt, not current live readiness.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

from ... import crypto


def verify_predecessor_assets(args, manifest: dict, metadata: dict, installed: bytes) -> None:
    diagnostic = {"command": ["assets", "status", "--json"],
                  "producer": "same_fixture_install_hydration", "verified": False}
    output = args.evidence_out.with_suffix(".diagnostic.json") if args.evidence_out else None
    try:
        maximum = args.predecessor_assets_maximum
        if type(maximum) is not int or maximum <= 0:
            raise ValueError("original assets receipt needs the paired byte budget")
        for digest in (args.predecessor_manifest_blake3, args.predecessor_manifest_sha256):
            if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
                raise ValueError("original assets receipt needs the host-bound manifest identities")
        if crypto.sha256(installed).hexdigest() != args.predecessor_manifest_sha256:
            raise ValueError("original assets manifest SHA256 differs from the paired input")
        if args.predecessor_assets_status is None:
            raise ValueError("original assets hydration receipt is missing")
        with args.predecessor_assets_status.open("rb") as stream:
            raw = stream.read(maximum + 1)
        diagnostic["bytes"] = len(raw)
        if len(raw) > maximum:
            raise ValueError("original assets receipt exceeds the paired byte budget")
        try:
            value = json.loads(raw)
        except (ValueError, UnicodeError) as error:
            raise ValueError("original assets receipt is not valid JSON") from error
        if not isinstance(value, dict):
            raise ValueError("original assets receipt must be an object")
        if value.get("ready") is not True or value.get("downloading") is not False:
            raise ValueError("original assets hydration receipt is not settled and ready")
        if value.get("error") is not None or value.get("reconcile_error") is not None:
            raise ValueError("original assets hydration receipt reports an error")
        reported = value.get("manifest")
        if not isinstance(reported, dict):
            raise ValueError("original assets receipt has no manifest object")
        assets = args.capsem_home / "assets"
        expected = {"path": str(assets / "manifest.json"),
                    "origin_path": str(assets / "manifest-metadata.json"),
                    "origin_source": args.metadata_manifest_url or args.manifest_url,
                    "origin": metadata.get("origin", "installed"),
                    "blake3": args.predecessor_manifest_blake3,
                    "validation_status": "valid", "validation_error": None,
                    "format": 2, "binaries_current": args.package_version}
        if any(reported.get(key) != value for key, value in expected.items()):
            raise ValueError("original assets manifest identity/provenance/validation differs")
        # Original `assets status` defaults to code, not an inferred modern runtime.
        profiles = manifest.get("profiles", {})
        profile = profiles.get("code", {}) if isinstance(profiles, dict) else {}
        arches = [row for row in profile.get("architectures", [])
                  if isinstance(row, dict) and row.get("architecture") == args.architecture]
        # f9's compact manifest view chooses default, else first lexical profile;
        # this channel pointer differs from the code assets command's descriptors.
        ordered = sorted(profiles, key=lambda name: (name != "default", name))
        current = profiles[ordered[0]]["architectures"][0]["image_revision"] if ordered else None
        if len(arches) != 1 or reported.get("assets_current") != current:
            raise ValueError("original assets receipt has a foreign profile architecture/revision")
        descriptors = arches[0].get("images", [])
        if {row.get("kind") for row in descriptors} != {"kernel", "initrd", "rootfs"} or len(descriptors) != 3:
            raise ValueError("original profile must declare its exact three asset descriptors")
        expected_assets = []
        for row in descriptors:
            name, digest = row["name"], row["digest"]["blake3"]
            if name != {"kernel": "vmlinuz", "initrd": "initrd.img", "rootfs": "rootfs.erofs"}[row["kind"]]:
                raise ValueError("original asset descriptor has a foreign logical name")
            stem, dot, extension = name.partition(".")
            resolved = f"{stem}-{digest[:16]}{dot}{extension}"
            expected_assets.append({"name": resolved, "status": "present",
                                    "path": str(assets / args.architecture / resolved)})
        observed = value.get("assets")
        if not isinstance(observed, list) or len(observed) != len(expected_assets) or any(
            observed.count(row) != 1 for row in expected_assets
        ):
            raise ValueError("original assets receipt differs from the selected descriptor family")
        diagnostic["verified"] = True
    except OSError as error:
        raise ValueError("original assets hydration receipt is unavailable") from error
    finally:
        if output is not None:
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps(diagnostic, sort_keys=True) + "\n")


def record_status_diagnostic(path: Path | None, status: str, code: int) -> None:
    """Retain only allowlisted command/result scalars, never CLI raw text."""
    if path is not None:
        path = path.with_suffix(".status-diagnostic.json")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({
            "command": ["status"], "exit": code,
            "installed": "Installed: true" in status,
            "running": "Running:   true" in status,
            "service_ok": "Service:   ok" in status,
            "gateway_ok": "Gateway:   ok" in status,
            "manifest_source_lines": sum(line.startswith("Manifest source:") for line in status.splitlines()),
            "manifest_status_lines": sum(line.startswith("Manifest status:") for line in status.splitlines()),
        }, sort_keys=True) + "\n")

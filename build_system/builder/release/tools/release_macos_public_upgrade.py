"""Public-predecessor evidence within the existing Tart Apply owner only.

No SQLite reads, migration, runtime conversion or new installed authority.
The dormant original file family is opaque integrity evidence of this disposable
account. Profile graph and modern runtime remain distinct domains.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

from .release_admission_refusal import file_identity, read_json
from .release_macos_pairing import _sha


def require_empty_vms(owner, *, timeout: float, maximum: int) -> None:
    response = owner.measure_request(
        "GET", "/vms/list", timeout=timeout, max_response_bytes=maximum
    )
    if response.status != 200 or json.loads(response.body).get("sandboxes") != []:
        raise ValueError("public upgrade requires genuine zero-prior-VM state")


def original_files(home: Path, *, maximum: int) -> dict:
    sessions = home / "sessions"
    if sessions.is_symlink() or not sessions.is_dir():
        raise ValueError("original sessions must be an owned regular directory")
    paths = {"main.db": sessions / "main.db"}
    for name in ("main.db-wal", "main.db-journal"):
        path = sessions / name
        if path.exists() or path.is_symlink():
            paths[name] = path
    bodies = sessions / "main.bodies"
    if bodies.is_symlink():
        raise ValueError("original archive directory must not be a symlink")
    if bodies.exists():
        for member in bodies.iterdir():
            paths[f"main.bodies/{member.name}"] = member
            if len(paths) > maximum:
                raise ValueError("original archive member count exceeds evidence budget")
    total = 0
    result = {}
    for name, path in paths.items():
        before = path.lstat()
        total += before.st_size
        if total > maximum:
            raise ValueError("original file family exceeds configured evidence bytes")
        result[name] = file_identity(path)
        after = path.lstat()
        if (
            before.st_dev,
            before.st_ino,
            before.st_size,
            before.st_mtime_ns,
            before.st_ctime_ns,
        ) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise ValueError("original file changed during upgrade snapshot")
    return result


def feed_selection(spec: dict, metadata: dict, manifest: Path, *, source: str) -> dict:
    if (
        metadata.get("manifest_url") != source
        or metadata.get("origin") != "update"
        or metadata.get("channel") != spec["channel"]
        or metadata.get("channel_locked") is not False
        or _sha(manifest) != spec["before"]["manifest_sha256"]
    ):
        raise ValueError("original CLI did not select the exact original feed bytes")
    return {
        "selected_feed": source,
        "original_embedded_url": spec["before"]["metadata_manifest_url"],
        "before_manifest_sha256": spec["before"]["manifest_sha256"],
        "selection": "original_cli_update_assets_manifest",
    }


def require_original_success(request: dict) -> None:
    if (
        request.get("outcome") != "succeeded"
        or request.get("attempts") != 1
        or request.get("transport") != "gateway_http"
        or type(request.get("http_status")) is not int
        or not 200 <= request["http_status"] < 300
    ):
        raise ValueError("public upgrade requires one complete successful original gateway Apply")


def validate_evidence(evidence: dict, spec: dict, *, source: str) -> None:
    if (
        evidence.get("scope") != "zero_prior_vm_public_upgrade"
        or evidence.get("zero_before_vms") is not True
        or evidence.get("original_files_preserved") is not True
    ):
        raise ValueError("public upgrade omitted original no-VM/preservation proof")
    selected = evidence.get("feed_selection", {})
    expected = {
        "selected_feed": source,
        "original_embedded_url": spec["before"]["metadata_manifest_url"],
        "before_manifest_sha256": spec["before"]["manifest_sha256"],
        "selection": "original_cli_update_assets_manifest",
    }
    if selected != expected or not evidence.get("files") or "main.db" not in evidence["files"]:
        raise ValueError("public upgrade has foreign feed or original-file evidence")
    for name, identity in evidence["files"].items():
        if name not in {"main.db", "main.db-wal", "main.db-journal"} and (
            not name.startswith("main.bodies/")
            or len(Path(name).parts) != 2
            or Path(name).parts[-1] in {".", ".."}
        ):
            raise ValueError("public upgrade has foreign original-file identity")
        if (
            not isinstance(identity, list)
            or len(identity) != 4
            or any(type(value) is not int or value < 0 for value in identity[:3])
            or not isinstance(identity[3], str)
            or re.fullmatch(r"[0-9a-f]{64}", identity[3]) is None
        ):
            raise ValueError("public upgrade original-file identity is malformed")
    if evidence.get("content_domains") != {
        phase: spec[phase]["content_identity"] for phase in ("before", "after")
    }:
        raise ValueError("public upgrade changed original content domains")


def write_evidence(path: Path, evidence: dict, maximum: int) -> None:
    serialized = (json.dumps(evidence, sort_keys=True) + "\n").encode()
    if len(serialized) > maximum:
        raise ValueError("public upgrade state receipt exceeds configured byte budget")
    path.write_bytes(serialized)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("capture", "preserved"))
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--home", required=True, type=Path)
    parser.add_argument("--source", required=True)
    args = parser.parse_args()
    spec = json.loads((args.root / "binary-apply-pair.json").read_text())
    if spec.get("scope") != "public_predecessor_upgrade":
        raise ValueError("public state helper requires explicit public upgrade scope")
    maximum = spec["limits"]["maximum"]
    output = args.root / "public-upgrade-state.json"
    if args.command == "capture":
        from helpers.http_transport import Transport

        owner = Transport(socket_path=str(args.home / "run/service.sock"))
        try:
            require_empty_vms(owner, timeout=spec["limits"]["timeout"], maximum=maximum)
        finally:
            owner.close()
        selected = feed_selection(
            spec,
            read_json(args.home / "assets" / "manifest-metadata.json", maximum),
            args.home / "assets" / "manifest.json",
            source=args.source,
        )
        evidence = {
            "scope": "zero_prior_vm_public_upgrade",
            "zero_before_vms": True,
            "feed_selection": selected,
            "files": original_files(args.home, maximum=maximum),
            "content_domains": {
                phase: spec[phase]["content_identity"] for phase in ("before", "after")
            },
        }
        if output.exists():
            raise ValueError("original public state receipt already exists")
    else:
        evidence = read_json(output, maximum)
        observed = original_files(args.home, maximum=maximum)
        unchanged = evidence["files"] == observed
        # Live SQLite checkpoint/audit writes can change this family normally.
        # Retain both observations for diagnosis; mismatch is not a corruption claim.
        write_evidence(
            args.root / "public-upgrade-observed-files.json",
            {
                "scope": "original_file_observation",
                "files": observed,
                "original_files_preserved": unchanged,
            },
            maximum,
        )
        if not unchanged:
            raise ValueError("original dormant file family changed during public upgrade")
        evidence["original_files_preserved"] = True
        validate_evidence(evidence, spec, source=args.source)
    write_evidence(output, evidence, maximum)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

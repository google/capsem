"""Scoped facts for the existing Tart owner's original-package safe refusal.

Existing glowup report/installed-health authority is reused. No SQLite query,
schema mutation, successful Apply claim or new product protocol is introduced.
Only the disposable synthetic host's fixed installed cohort and DB/WAL payload
are fingerprinted; SQLite SHM coordination is deliberately excluded.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import stat
import subprocess
from pathlib import Path

from ... import crypto
from .release_glowup import (
    REPORT_SCHEMA,
    ArtifactIdentity,
    build_report,
    validate_installed_evidence,
)

ADAPTER = "macos-tart-host-admission-refusal"
SCOPE = "incompatible_host_preinstall_refusal"


def read_json(path: Path, maximum: int) -> dict:
    with path.open("rb") as stream:
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError("host refusal evidence exceeds existing response budget")
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValueError("host refusal evidence must be an object")
    return value


def file_identity(path: Path) -> list:
    """Fingerprint only this disposable fixture's original preservation bytes.

    SHA-256 is an intentional test-staging exception: clean Tart runs stdlib-only
    Python 3.9 with -S, without the native BLAKE3 binding. Reuse the existing
    package/cohort SHA-256 owner rather than adding guest dependencies or copying
    ledger snapshots. This bounded check is outside product/poll/benchmark paths;
    production integrity/checksum owners remain accelerated BLAKE3/XXH3.
    """
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    except OSError as error:
        raise ValueError(f"host refusal requires a regular original file: {path}") from error
    with os.fdopen(fd, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError("host refusal requires regular original files")
        digest, remaining = crypto.sha256(), metadata.st_size
        while remaining:
            data = stream.read(min(1024 * 1024, remaining))
            if not data:
                raise ValueError("original file changed during refusal snapshot")
            digest.update(data)
            remaining -= len(data)
        if stream.read(1):
            raise ValueError("original file grew during refusal snapshot")
    return [metadata.st_dev, metadata.st_ino, metadata.st_size, digest.hexdigest()]


def required_files(names: list[str]) -> set[str]:
    return {"host.db", "manifest", "metadata", "plist", "sentinel", "app", "app-info"} | {
        f"{kind}/{name}" for kind in ("bin", "payload") for name in names
    }


def snapshot(home: Path, spec: dict) -> dict:
    names = spec["before"]["binaries"]
    files = {
        **{f"bin/{name}": home / "bin" / name for name in names},
        **{f"payload/{name}": Path("/usr/local/share/capsem/bin") / name for name in names},
        "app": Path("/Applications/Capsem.app/Contents/MacOS/capsem-app"),
        "app-info": Path("/Applications/Capsem.app/Contents/Info.plist"),
        "host.db": home / "sessions/host.db",
        "manifest": home / "assets" / "manifest.json",
        "metadata": home / "assets" / "manifest-metadata.json",
        "plist": home.parent / "Library/LaunchAgents/com.capsem.service.plist",
        "sentinel": home / "preinstall-preservation.txt",
    }
    wal = home / "sessions/host.db-wal"
    if wal.exists():
        files["host.db-wal"] = wal
    # Integrity evidence of this synthetic original host only; no archive
    # decoding, record queries or product admission body scan is performed.
    for member in (home / "sessions/host.bodies").iterdir():
        files[f"archive/{member.name}"] = member
    processes = {}
    for role in ("service", "gateway"):
        pid = (home / "run" / f"{role}.pid").read_text().strip()
        if not pid.isdecimal() or int(pid) <= 0:
            raise ValueError("host refusal requires original live process IDs")
        identity = subprocess.check_output(
            ["/bin/ps", "-p", pid, "-o", "pid=,uid=,lstart=,comm="],
            text=True,
        ).strip()
        executable = str(home / "bin" / f"capsem-{role}")
        if not identity or identity.split()[0] != pid or not identity.endswith(executable):
            raise ValueError("original process is not the selected installed cohort")
        processes[role] = identity
    return {
        "files": {name: file_identity(path) for name, path in files.items()},
        "processes": processes,
        "receipt": subprocess.check_output(
            ["/usr/sbin/pkgutil", "--pkg-info", "com.capsem.pkg"], text=True
        ),
        "root_stages": sum(1 for _ in Path("/var/tmp").glob("capsem-preinstall.*")),
    }


def validate_report(report: dict, spec: dict) -> dict:
    if set(report) != {
        "schema",
        "adapter",
        "artifact",
        "installed",
        "capabilities",
        "adapter_evidence",
    }:
        raise ValueError("host refusal cannot inherit other proof fields")
    if report.get("schema") != REPORT_SCHEMA or report.get("adapter") != ADAPTER:
        raise ValueError("foreign host-refusal report scope")
    if report.get("artifact") != spec["before"]["artifact"]:
        raise ValueError("host refusal must report the original installed artifact")
    installed = validate_installed_evidence(report.get("installed", {}))
    if (
        installed["package_version"] != spec["before"]["artifact"]["version"]
        or installed["manifest_url"] != spec["source"]
        or installed["channel"] != spec["channel"]
    ):
        raise ValueError("host refusal lost original positive installed health")
    evidence = report.get("adapter_evidence", {})
    if validate_installed_evidence(evidence.get("installed_before", {})) != installed:
        raise ValueError("original positive installed health changed")
    if any(
        evidence.get(key) != expected
        for key, expected in (
            ("scope", SCOPE),
            ("before", spec["before"]),
            ("candidate", spec["candidate"]),
            ("reason", "obsolete_host_hash_chain"),
            ("admission_phase", "admit_candidate_ledger"),
        )
    ):
        raise ValueError("foreign candidate or admission-refusal evidence")
    status = evidence.get("installer_status")
    if type(status) is not int or not 0 < status <= 255:
        raise ValueError("host refusal requires one failed original installer")
    before, after = evidence.get("snapshot_before", {}), evidence.get("snapshot_after", {})
    expected = {"files", "processes", "receipt", "root_stages"}
    comparable = {**after, "files": dict(after.get("files", {}))}
    # SQLite READ_ONLY may create an EMPTY WAL plus SHM coordination. Existing
    # WAL payload is immutable here; never pretend all sidecar bytes are static.
    if "host.db-wal" not in before.get("files", {}) and "host.db-wal" in comparable["files"]:
        if comparable["files"]["host.db-wal"][2:] != [0, crypto.sha256(b"").hexdigest()]:
            raise ValueError("candidate appended new WAL payload")
        del comparable["files"]["host.db-wal"]
    if set(before) != expected or set(after) != expected or before != comparable:
        raise ValueError("original package/process/data changed during refusal")
    if not required_files(spec["before"]["binaries"]).issubset(before["files"]):
        raise ValueError("host refusal omitted original data or cohort")
    if not any(name.startswith("archive/") for name in before["files"]):
        raise ValueError("host refusal omitted original archive bytes")
    for identity in before["files"].values():
        if (
            not isinstance(identity, list)
            or len(identity) != 4
            or any(type(value) is not int or value < 0 for value in identity[:3])
            or not isinstance(identity[3], str)
            or re.fullmatch(r"[0-9a-f]{64}", identity[3]) is None
        ):
            raise ValueError("invalid original file integrity evidence")
    if (
        set(before["processes"]) != {"service", "gateway"}
        or not all(before["processes"].values())
        or not before["receipt"]
    ):
        raise ValueError("host refusal omitted original live identity")
    if type(before["root_stages"]) is not int or before["root_stages"] != 0:
        raise ValueError("host refusal left candidate root staging")
    capabilities = report.get("capabilities", {})
    if (
        capabilities.get("native_install") is not True
        or capabilities.get("launchd") is not True
        or any(
            capabilities.get(key) is True
            for key in ("physical_vz_boot", "full_doctor", "installed_winterfell")
        )
    ):
        raise ValueError("host refusal cannot credit unproved native capabilities")
    return report


def finish(root: Path, home: Path, spec: dict, maximum: int) -> dict:
    log = (home / "logs/install-latest.log").resolve(strict=True)
    if log.parent != home / "logs":
        raise ValueError("candidate install log escaped selected fixture")
    with log.open("rb") as stream:
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError("candidate install log exceeds existing response budget")
    # Preserve ONLY the selected candidate's bounded synthetic log before any
    # assertion, including failure; combined earlier installation logs are not evidence.
    (root / "host-refusal-candidate-install.log").write_bytes(raw)
    if b"obsolete host hash-chain schema" not in raw:
        raise ValueError("candidate did not prove incompatible HOST schema refusal")
    if b"Failed phase: admit_candidate_ledger" not in raw.splitlines() or any(
        marker in raw
        for marker in (
            b"event=unload_launch_agent",
            b"event=remove_old_payload",
            b"event=remove_user_app_payload",
        )
    ):
        raise ValueError("candidate reached retirement before refusing")
    installed = read_json(root / "host-refusal-after-installed.json", maximum)
    installed.update(package_receipt=True, binary_cohort=True)
    installed_before = read_json(root / "host-refusal-before-installed.json", maximum)
    installed_before.update(package_receipt=True, binary_cohort=True)
    artifact = ArtifactIdentity.from_path(
        root / spec["package"],
        version=spec["before"]["artifact"]["version"],
        platform="macos",
        architecture="arm64",
    )
    from macos_tart_transition_support import local_tart_capabilities

    report = build_report(
        adapter=ADAPTER,
        artifact=artifact,
        installed=installed,
        capabilities=local_tart_capabilities(),
    )
    report["adapter_evidence"] = {
        "scope": SCOPE,
        "before": spec["before"],
        "candidate": spec["candidate"],
        "installer_status": int((root / "host-refusal-status.txt").read_text()),
        "reason": "obsolete_host_hash_chain",
        "admission_phase": "admit_candidate_ledger",
        "installed_before": installed_before,
        "snapshot_before": read_json(root / "host-refusal-snapshot.json", maximum),
        "snapshot_after": snapshot(home, spec),
    }
    return validate_report(report, spec)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("binaries", "snapshot", "finish"))
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--home", required=True, type=Path)
    parser.add_argument("--maximum", required=True, type=int)
    args = parser.parse_args()
    if args.maximum <= 0:
        parser.error("maximum must be the positive existing response budget")
    spec = read_json(args.root / "host-admission-inputs.json", args.maximum)
    if args.command == "binaries":
        print("\n".join(spec["before"]["binaries"]))
    else:
        result = (
            snapshot(args.home, spec)
            if args.command == "snapshot"
            else finish(args.root, args.home, spec, args.maximum)
        )
        name = "host-refusal-snapshot.json" if args.command == "snapshot" else "report.json"
        (args.root / name).write_text(json.dumps(result, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

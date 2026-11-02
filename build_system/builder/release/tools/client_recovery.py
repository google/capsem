"""Continue CI publication of attested archives after native release completion.

The controller revision owns publication tooling; the original run owns the
qualified payload. Never substitute either identity for GITHUB_SHA.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any

from ...gate.config import GateConfig, load
from .client_package_archives import Package
from .verify_channel_downloads import fetch
from .verify_immutable_publication import file_identity
from .verify_release_recovery_run import verify_client_recovery_run

CLIENT_SOURCE_PATHS = ("sdk", "mcp", "integrations", "Cargo.toml")
RUN_FIELDS = "databaseId,headSha,headBranch,event,workflowName,status,conclusion,jobs"


def capture(root: Path, *argv: str) -> str:
    return subprocess.check_output(argv, cwd=root, text=True).strip()


def original_run(root: Path, run_id: str, tag: str) -> str:
    if not run_id.isdecimal():
        raise ValueError("client recovery requires a numeric original run id")
    run = json.loads(capture(root, "gh", "run", "view", run_id, "--repo", "google/capsem", "--json", RUN_FIELDS))
    source = capture(root, "git", "rev-parse", f"{tag}^{{commit}}")
    verify_client_recovery_run(run, run_id, source)
    release = json.loads(capture(root, "gh", "release", "view", tag, "--repo", "google/capsem",
                                 "--json", "isDraft,isPrerelease,targetCommitish,tagName"))
    if release != {"isDraft": False, "isPrerelease": False, "targetCommitish": source, "tagName": tag}:
        raise ValueError("client recovery requires the already-public exact stable native release")
    # Root Cargo.lock audits the controller/native workspace. Enabled Python/npm
    # archives retain every client lock under these roots; Rust publication is
    # deferred (#382), and original Rust archives are never rebuilt here.
    if capture(root, "git", "diff", "--name-only", source, "HEAD", "--", *CLIENT_SOURCE_PATHS):
        raise ValueError("client recovery may not change qualified client source or dependency locks")
    return source


def verify_archives(config: GateConfig, packages: tuple[Package, ...], source: str) -> None:
    for package in packages:
        if package.registry == "npm":
            for archive in package.archives:
                subprocess.run(["gh", "attestation", "verify", str(archive), "--repo", "google/capsem",
                                "--source-digest", source, "--deny-self-hosted-runners", "--signer-workflow",
                                "google/capsem/.github/workflows/release-client-packages.yaml"],
                               cwd=config.root, check=True)


def verify_native(path: Path, manifest: dict[str, Any], source: str) -> None:
    rows = [row for row in manifest.get("packages", []) if row.get("name") == path.name
            and row.get("source_commit") == source and row.get("platform") == "linux"]
    if manifest.get("channel") != "stable" or len(rows) != 1:
        raise ValueError("native client fixture is not selected by the live stable manifest")
    row = rows[0]
    if file_identity(path) != (row["bytes"], row["digest"]["sha256"]):
        raise ValueError("native client fixture differs from the public qualified package")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--native-dir", type=Path)
    args = parser.parse_args()
    config = load(Path.cwd())
    source = original_run(config.root, args.run_id, args.release_tag)
    if args.native_dir is None:
        print(f"Verified original native release run {args.run_id} source {source}")
        return
    packages = list(args.native_dir.glob("*.deb"))
    if len(packages) != 1:
        raise ValueError("client recovery requires exactly one original native fixture")
    manifest = json.loads(fetch("https://release.capsem.org/assets/stable/manifest.json"))
    verify_native(packages[0], manifest, source)


if __name__ == "__main__":
    main()

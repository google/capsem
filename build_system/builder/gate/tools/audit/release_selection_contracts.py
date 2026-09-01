"""Repository-wide invariants used by the release-selection guard."""

from __future__ import annotations

import os
import re
import subprocess
import sys
from collections.abc import Iterable
from pathlib import Path


def repository_files(root: Path) -> Iterable[Path]:
    """Yield tracked and non-ignored untracked files without requiring rg."""
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-co", "--exclude-standard", "-z"],
            check=True,
            capture_output=True,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        ignored_parts = {
            ".astro",
            ".claude",
            "cache",
            ".git",
            ".venv",
            "dist",
            "node_modules",
            "target",
        }
        yield from (
            path
            for path in root.rglob("*")
            if path.is_file()
            and not ignored_parts.intersection(path.relative_to(root).parts)
            and not any(part.startswith(".sprinty") for part in path.relative_to(root).parts)
        )
        return

    for raw in result.stdout.split(b"\0"):
        if not raw:
            continue
        path = root / os.fsdecode(raw)
        relative = path.relative_to(root)
        if any(part.startswith(".sprinty") for part in relative.parts):
            continue
        if any(
            part in {".astro", ".claude", "cache", "dist", "node_modules", "target"}
            for part in relative.parts
        ):
            continue
        if path.is_file():
            yield path


def _builtin_profiles_match(root: Path) -> bool:
    configured = sorted(
        path.parent.name for path in (root / "config/profiles").glob("*/profile.toml")
    )
    contract = root / "crates/capsem-core/src/net/policy_config/profile_contract.rs"
    embedded = sorted(
        set(
            re.findall(
                r"config/profiles/([^/]+)/profile\.toml",
                contract.read_text(encoding="utf-8"),
            )
        )
    )
    if configured == embedded:
        return False
    print("ERROR: builtin_profile_configs does not exactly mirror config/profiles", file=sys.stderr)
    print("configured profiles:", file=sys.stderr)
    print("\n".join(configured), file=sys.stderr)
    print("embedded profiles:", file=sys.stderr)
    print("\n".join(embedded), file=sys.stderr)
    return True


def _retired_doctrine_reintroduced(root: Path) -> bool:
    markers = (
        "release-" + "qualification.yaml",
        "check-release-" + "qualification.py",
        "qualify-" + "release",
        "cut-" + "release",
    )
    resurrected: list[str] = []
    for path in repository_files(root):
        relative = path.relative_to(root)
        if relative.as_posix() == "CHANGELOG.md":
            continue
        data = path.read_bytes()
        if b"\0" in data:
            continue
        contents = data.decode("utf-8", errors="replace")
        for marker in markers:
            if marker in contents:
                line = contents.count("\n", 0, contents.index(marker)) + 1
                resurrected.append(f"{relative}:{line}:{marker}")
    if not resurrected:
        return False
    print("ERROR: retired independent release doctrine was reintroduced", file=sys.stderr)
    print("\n".join(resurrected), file=sys.stderr)
    return True


def _release_workflows_are_serialized(root: Path) -> bool:
    workflows = root / ".github/workflows"
    release_workflows = [
        workflows / name
        for name in ("release.yaml", "release-assets.yaml", "release-publication-recovery.yaml")
    ]
    failed = False
    expected_group = "group: capsem-release-${{ inputs.channel }}"
    for workflow in release_workflows:
        contents = workflow.read_text(encoding="utf-8")
        if expected_group not in contents or "cancel-in-progress: false" not in contents:
            print(
                f"ERROR: {workflow.relative_to(root)} does not use the shared per-channel "
                "release lock",
                file=sys.stderr,
            )
            failed = True

    allowed_writers = {path.resolve() for path in release_workflows}
    writer_markers = (
        "stage-profile-publication.py",
        "capsem-admin -- release",
        'gh release upload "$RELEASE_TAG" "$named"',
    )
    for workflow in workflows.glob("*.yaml"):
        if workflow.resolve() in allowed_writers:
            continue
        contents = workflow.read_text(encoding="utf-8")
        if any(marker in contents for marker in writer_markers):
            print(
                "ERROR: production source-manifest writer outside serialized release "
                f"workflows: {workflow.relative_to(root)}",
                file=sys.stderr,
            )
            failed = True

    deploy_call = "uses: ./.github/workflows/release-channel.yaml"
    for workflow in workflows.glob("*.yaml"):
        contents = workflow.read_text(encoding="utf-8")
        if deploy_call not in contents or workflow.resolve() in allowed_writers:
            continue
        if workflow.name != "release-channel-staging.yaml":
            print(
                "ERROR: production deploy caller outside serialized release workflows: "
                f"{workflow.relative_to(root)}",
                file=sys.stderr,
            )
            failed = True
        elif (
            "deploy_branch: ${{ inputs.deploy_branch }}" not in contents
            or "validate_complete_public_channels: false" not in contents
        ):
            print(
                "ERROR: release-channel staging caller is not constrained to preview mode",
                file=sys.stderr,
            )
            failed = True
    return failed

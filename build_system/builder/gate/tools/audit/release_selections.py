"""Reject hardcoded release channels and profile selections.

This guard intentionally uses only the Python standard library. It runs before
Capsem's expensive test stages and therefore must work in the same clean Linux
release environment without assuming developer tools such as ripgrep.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from collections.abc import Iterable
from pathlib import Path

from capsem_builder.gate import project_root

PROFILE_TERMS = r"(?:code|co-work|cowork|terminal|termional|gui)"
MATCH_GUARDS = (
    (
        "user-facing session request hardcodes a named profile",
        rf"profile_id\s*:\s*['\"]{PROFILE_TERMS}['\"]",
        ("frontend/src/lib/components", "crates/capsem-tray/src"),
    ),
    (
        "profile picker fabricates a named profile instead of using the installed catalog",
        rf"(?:profileId\s*=[^\n]*['\"]{PROFILE_TERMS}['\"]|"
        rf"<option[^>]*value=['\"]{PROFILE_TERMS}['\"])",
        ("frontend/src/lib/components",),
    ),
    (
        "MCP request bypasses its explicit profile parameter",
        r"['\"]profile_id['\"]\s*:\s*DEFAULT_PROFILE_ID",
        ("crates/capsem-mcp/src/main.rs",),
    ),
    (
        "profile-scoped MCP route silently uses the default profile",
        r"['\"]/profiles/\{\}/mcp[^;]{0,240}DEFAULT_PROFILE_ID",
        ("crates/capsem/src/main.rs", "crates/capsem-mcp/src/main.rs"),
    ),
    (
        "workflow input silently defaults a profile or public release channel",
        rf"(?:channel|asset_channel|profile):\s*\n(?:[^\n]*\n){{0,8}}"
        rf"\s*default:\s*(?:{PROFILE_TERMS}|stable|nightly)\s*\n",
        (".github/workflows",),
    ),
    (
        "binary release packaging materializes one named profile instead of the selected "
        "channel catalog",
        rf"--profile\s+\S*{PROFILE_TERMS}",
        (".github/workflows/release.yaml",),
    ),
    (
        "release workflow hardcodes a stable/nightly ASSET_MANIFEST_URL instead of an "
        "explicit channel input",
        r"ASSET_MANIFEST_URL:.*assets/(?:stable|nightly)/manifest\.json",
        (".github/workflows",),
    ),
    (
        "reusable release deployment makes its channel optional",
        r"channel:\s*\n(?:[^\n]*\n){0,3}\s*required:\s*false",
        (".github/workflows/release-channel.yaml",),
    ),
    (
        "reusable release deployment silently substitutes stable for its channel input",
        r"inputs\.channel\s*\|\|\s*['\"]stable['\"]",
        (".github/workflows/release-channel.yaml",),
    ),
    (
        "native postinstall silently falls back to a public channel",
        r"MANIFEST_SOURCE=['\"]https://release\.capsem\.org/assets/(?:stable|nightly)/"
        r"manifest\.json['\"]",
        ("scripts/deb-postinst.sh", "scripts/pkg-scripts/postinstall"),
    ),
    (
        "native postinstall bypasses installed manifest-metadata provenance",
        r"CAPSEM_RELEASE_(?:MANIFEST|HEALTH)_URL",
        ("scripts/deb-postinst.sh", "scripts/pkg-scripts/postinstall"),
    ),
    (
        "installed update test bypasses manifest-metadata provenance",
        r"['\"]CAPSEM_RELEASE_(?:MANIFEST|HEALTH)_URL['\"]\s*:",
        ("tests/capsem-install",),
    ),
    (
        "legacy split manifest/update sidecar was reintroduced",
        r"manifest-origin\.json|update-check\.json",
        (
            "scripts/build-pkg.sh",
            "scripts/repack-deb.sh",
            "scripts/deb-postinst.sh",
            "scripts/pkg-scripts/postinstall",
            "crates/capsem/src/update.rs",
            "crates/capsem-service/src/main.rs",
        ),
    ),
    (
        "public release HTTP reader passes a bare URL to urllib and may be rejected by "
        "the edge",
        r"urlopen\(\s*(?:source|url|manifest_url)\s*,",
        (
            "scripts/materialize-config.sh",
            "scripts/build-complete-release-channel.py",
            "scripts/local-release-glowup.py",
        ),
    ),
    (
        "installed update flow silently substitutes the stable manifest when source "
        "metadata is absent",
        r"unwrap_or(?:_else)?\([^\n]*DEFAULT_RELEASE_MANIFEST_URL",
        ("crates/capsem/src/update.rs",),
    ),
)


def default_root() -> Path:
    configured = os.environ.get("CAPSEM_GUARD_ROOT")
    return Path(configured) if configured else project_root()


def source_files(root: Path, paths: Iterable[str]) -> Iterable[Path]:
    for relative in paths:
        path = root / relative
        if path.is_file():
            yield path
        elif path.is_dir():
            yield from (candidate for candidate in path.rglob("*") if candidate.is_file())


def reject_matches(root: Path, label: str, pattern: str, paths: Iterable[str]) -> bool:
    regex = re.compile(pattern, re.MULTILINE)
    matches: list[str] = []
    for path in source_files(root, paths):
        contents = path.read_text(encoding="utf-8", errors="replace")
        for match in regex.finditer(contents):
            line = contents.count("\n", 0, match.start()) + 1
            excerpt = match.group(0).replace("\n", "\\n")
            matches.append(f"{path.relative_to(root)}:{line}:{excerpt}")
    if not matches:
        return False
    print(f"ERROR: {label}", file=sys.stderr)
    print("\n".join(matches), file=sys.stderr)
    return True


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
            part in {".astro", ".claude", "dist", "node_modules", "target"}
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


def main(root: Path | None = None) -> int:
    root = (root or default_root()).resolve()
    failed = False
    for label, pattern, paths in MATCH_GUARDS:
        failed = reject_matches(root, label, pattern, paths) or failed
    failed = _builtin_profiles_match(root) or failed
    failed = _retired_doctrine_reintroduced(root) or failed
    failed = _release_workflows_are_serialized(root) or failed
    if failed:
        return 1
    print("Hardcoded profile/channel selection guard passed.")
    return 0

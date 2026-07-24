#!/usr/bin/env python3
"""Reject hardcoded release channels and profile selections.

This guard intentionally uses only the Python standard library. It runs before
Capsem's expensive test stages and therefore must work in the same clean Linux
release environment without assuming developer tools such as
ripgrep are installed.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from collections.abc import Iterable
from pathlib import Path


ROOT = Path(os.environ.get("CAPSEM_GUARD_ROOT", Path(__file__).resolve().parent.parent))
PROFILE_TERMS = r"(?:code|co-work|cowork|terminal|termional|gui)"
failed = False


def source_files(paths: Iterable[str]) -> Iterable[Path]:
    for relative in paths:
        path = ROOT / relative
        if path.is_file():
            yield path
        elif path.is_dir():
            yield from (candidate for candidate in path.rglob("*") if candidate.is_file())


def reject_matches(label: str, pattern: str, *paths: str) -> None:
    global failed
    regex = re.compile(pattern, re.MULTILINE)
    matches: list[str] = []
    for path in source_files(paths):
        contents = path.read_text(encoding="utf-8", errors="replace")
        for match in regex.finditer(contents):
            line = contents.count("\n", 0, match.start()) + 1
            excerpt = match.group(0).replace("\n", "\\n")
            matches.append(f"{path.relative_to(ROOT)}:{line}:{excerpt}")
    if matches:
        print(f"ERROR: {label}", file=sys.stderr)
        print("\n".join(matches), file=sys.stderr)
        failed = True


reject_matches(
    "user-facing session request hardcodes a named profile",
    rf"profile_id\s*:\s*['\"]{PROFILE_TERMS}['\"]",
    "frontend/src/lib/components",
    "crates/capsem-tray/src",
)

reject_matches(
    "profile picker fabricates a named profile instead of using the installed catalog",
    rf"(?:profileId\s*=[^\n]*['\"]{PROFILE_TERMS}['\"]|"
    rf"<option[^>]*value=['\"]{PROFILE_TERMS}['\"])",
    "frontend/src/lib/components",
)

reject_matches(
    "MCP request bypasses its explicit profile parameter",
    r"['\"]profile_id['\"]\s*:\s*DEFAULT_PROFILE_ID",
    "crates/capsem-mcp/src/main.rs",
)

reject_matches(
    "profile-scoped MCP route silently uses the default profile",
    r"['\"]/profiles/\{\}/mcp[^;]{0,240}DEFAULT_PROFILE_ID",
    "crates/capsem/src/main.rs",
    "crates/capsem-mcp/src/main.rs",
)

configured_profiles = sorted(
    path.parent.name for path in (ROOT / "config/profiles").glob("*/profile.toml")
)
profile_contract = ROOT / "crates/capsem-core/src/net/policy_config/profile_contract.rs"
embedded_profiles = sorted(
    set(
        re.findall(
            r"config/profiles/([^/]+)/profile\.toml",
            profile_contract.read_text(encoding="utf-8"),
        )
    )
)
if configured_profiles != embedded_profiles:
    print("ERROR: builtin_profile_configs does not exactly mirror config/profiles", file=sys.stderr)
    print("configured profiles:", file=sys.stderr)
    print("\n".join(configured_profiles), file=sys.stderr)
    print("embedded profiles:", file=sys.stderr)
    print("\n".join(embedded_profiles), file=sys.stderr)
    failed = True

reject_matches(
    "workflow input silently defaults a profile or public release channel",
    rf"(?:channel|asset_channel|profile):\s*\n(?:[^\n]*\n){{0,8}}"
    rf"\s*default:\s*(?:{PROFILE_TERMS}|stable|nightly)\s*\n",
    ".github/workflows",
)

reject_matches(
    "binary release packaging materializes one named profile instead of the selected channel catalog",
    rf"--profile\s+\S*{PROFILE_TERMS}",
    ".github/workflows/release.yaml",
)

reject_matches(
    "release workflow hardcodes a stable/nightly ASSET_MANIFEST_URL instead of an explicit channel input",
    r"ASSET_MANIFEST_URL:.*assets/(?:stable|nightly)/manifest\.json",
    ".github/workflows",
)

reject_matches(
    "reusable release deployment makes its channel optional",
    r"channel:\s*\n(?:[^\n]*\n){0,3}\s*required:\s*false",
    ".github/workflows/release-channel.yaml",
)

reject_matches(
    "reusable release deployment silently substitutes stable for its channel input",
    r"inputs\.channel\s*\|\|\s*['\"]stable['\"]",
    ".github/workflows/release-channel.yaml",
)

reject_matches(
    "native postinstall silently falls back to a public channel",
    r"MANIFEST_SOURCE=['\"]https://release\.capsem\.org/assets/(?:stable|nightly)/manifest\.json['\"]",
    "scripts/deb-postinst.sh",
    "scripts/pkg-scripts/postinstall",
)

reject_matches(
    "native postinstall bypasses installed manifest-metadata provenance",
    r"CAPSEM_RELEASE_(?:MANIFEST|HEALTH)_URL",
    "scripts/deb-postinst.sh",
    "scripts/pkg-scripts/postinstall",
)

reject_matches(
    "installed update test bypasses manifest-metadata provenance",
    r"['\"]CAPSEM_RELEASE_(?:MANIFEST|HEALTH)_URL['\"]\s*:",
    "tests/capsem-install",
)

reject_matches(
    "legacy split manifest/update sidecar was reintroduced",
    r"manifest-origin\.json|update-check\.json",
    "scripts/build-pkg.sh",
    "scripts/repack-deb.sh",
    "scripts/deb-postinst.sh",
    "scripts/pkg-scripts/postinstall",
    "crates/capsem/src/update.rs",
    "crates/capsem-service/src/main.rs",
)

reject_matches(
    "public release HTTP reader passes a bare URL to urllib and may be rejected by the edge",
    r"urlopen\(\s*(?:source|url|manifest_url)\s*,",
    "scripts/materialize-config.sh",
    "scripts/build-complete-release-channel.py",
    "scripts/local-release-glowup.py",
)

reject_matches(
    "installed update flow silently substitutes the stable manifest when source metadata is absent",
    r"unwrap_or(?:_else)?\([^\n]*DEFAULT_RELEASE_MANIFEST_URL",
    "crates/capsem/src/update.rs",
)

def repository_files() -> Iterable[Path]:
    """Yield tracked and non-ignored untracked files without requiring rg."""

    try:
        result = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "-co", "--exclude-standard", "-z"],
            check=True,
            capture_output=True,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        ignored_parts = {".astro", ".git", ".venv", "dist", "node_modules", "target"}
        yield from (
            path
            for path in ROOT.rglob("*")
            if path.is_file()
            and not ignored_parts.intersection(path.relative_to(ROOT).parts)
            and not any(
                part.startswith(".sprinty") for part in path.relative_to(ROOT).parts
            )
        )
        return

    for raw in result.stdout.split(b"\0"):
        if raw:
            path = ROOT / os.fsdecode(raw)
            relative = path.relative_to(ROOT)
            if any(part.startswith(".sprinty") for part in relative.parts):
                continue
            if any(part in {".astro", "dist", "node_modules", "target"} for part in relative.parts):
                continue
            if path.is_file():
                yield path


retired_markers = (
    "release-" + "qualification.yaml",
    "check-release-" + "qualification.py",
    "qualify-" + "release",
    "cut-" + "release",
)
retired_sha_pattern = re.compile(r"\bexact[- ]" + r"SHA\b", re.IGNORECASE)
resurrected: list[str] = []
for path in repository_files():
    relative = path.relative_to(ROOT)
    if relative.as_posix() == "CHANGELOG.md":
        continue
    data = path.read_bytes()
    if b"\0" in data:
        continue
    contents = data.decode("utf-8", errors="replace")
    for marker in retired_markers:
        if marker in contents:
            line = contents.count("\n", 0, contents.index(marker)) + 1
            resurrected.append(f"{relative}:{line}:{marker}")
    match = retired_sha_pattern.search(contents)
    if match:
        line = contents.count("\n", 0, match.start()) + 1
        resurrected.append(f"{relative}:{line}:{match.group(0)}")

if resurrected:
    print("ERROR: retired independent release doctrine was reintroduced", file=sys.stderr)
    print("\n".join(resurrected), file=sys.stderr)
    failed = True


release_workflows = [
    ROOT / ".github/workflows/release.yaml",
    ROOT / ".github/workflows/release-assets.yaml",
]
expected_group = "group: capsem-release-${{ inputs.channel }}"
for workflow in release_workflows:
    contents = workflow.read_text(encoding="utf-8")
    if expected_group not in contents or "cancel-in-progress: false" not in contents:
        print(
            f"ERROR: {workflow.relative_to(ROOT)} does not use the shared per-channel release lock",
            file=sys.stderr,
        )
        failed = True

allowed_writers = {path.resolve() for path in release_workflows}
writer_markers = (
    "stage-profile-publication.py",
    "capsem-admin -- release",
    "gh release upload \"$RELEASE_TAG\" \"$named\"",
)
for workflow in (ROOT / ".github/workflows").glob("*.yaml"):
    if workflow.resolve() in allowed_writers:
        continue
    contents = workflow.read_text(encoding="utf-8")
    if any(marker in contents for marker in writer_markers):
        print(
            f"ERROR: production source-manifest writer outside serialized release workflows: "
            f"{workflow.relative_to(ROOT)}",
            file=sys.stderr,
        )
        failed = True

deploy_call = "uses: ./.github/workflows/release-channel.yaml"
for workflow in (ROOT / ".github/workflows").glob("*.yaml"):
    contents = workflow.read_text(encoding="utf-8")
    if deploy_call not in contents or workflow.resolve() in allowed_writers:
        continue
    if workflow.name != "release-channel-staging.yaml":
        print(
            f"ERROR: production deploy caller outside serialized release workflows: "
            f"{workflow.relative_to(ROOT)}",
            file=sys.stderr,
        )
        failed = True
    elif (
        "deploy_branch: ${{ inputs.deploy_branch }}" not in contents
        or "validate_complete_public_channels: false" not in contents
    ):
        print("ERROR: release-channel staging caller is not constrained to preview mode", file=sys.stderr)
        failed = True

if failed:
    raise SystemExit(1)

print("Hardcoded profile/channel selection guard passed.")

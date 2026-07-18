#!/usr/bin/env python3
"""Reject hardcoded release channels and profile selections.

This guard intentionally uses only the Python standard library. It runs before
Capsem's expensive test stages and therefore must work in the same clean Linux
environment as release qualification without assuming developer tools such as
ripgrep are installed.
"""

from __future__ import annotations

import os
import re
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
    "release packaging materializes one named profile instead of the catalog",
    rf"--profile\s+\S*{PROFILE_TERMS}",
    ".github/workflows/release.yaml",
)

reject_matches(
    "workflow input silently defaults a profile or public release channel",
    rf"(?:channel|asset_channel|profile):\s*\n(?:[^\n]*\n){{0,8}}"
    rf"\s*default:\s*(?:{PROFILE_TERMS}|stable|nightly)\s*\n",
    ".github/workflows",
)

reject_matches(
    "release qualification hardcodes stable/nightly instead of its channel input",
    r"CAPSEM_INSTALL_(?:MANIFEST_URL|CHANNEL):.*(?:stable|nightly)",
    ".github/workflows/release-qualification.yaml",
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
    "release qualification bypasses installed manifest-metadata provenance",
    r"CAPSEM_RELEASE_(?:MANIFEST|HEALTH)_URL=",
    ".github/workflows/release-qualification.yaml",
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
    "installed update flow silently substitutes the stable manifest when source metadata is absent",
    r"unwrap_or(?:_else)?\([^\n]*DEFAULT_RELEASE_MANIFEST_URL",
    "crates/capsem/src/update.rs",
)

qualification_paths = ("justfile", ".github/workflows/release.yaml")
missing_channel: list[str] = []
for path in source_files(qualification_paths):
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if "check-release-qualification.py" in line and "--channel" not in line:
            missing_channel.append(f"{path.relative_to(ROOT)}:{line_number}:{line}")
if missing_channel:
    print("ERROR: release qualification check is not bound to an explicit channel", file=sys.stderr)
    print("\n".join(missing_channel), file=sys.stderr)
    failed = True

if failed:
    raise SystemExit(1)

print("Hardcoded profile/channel selection guard passed.")

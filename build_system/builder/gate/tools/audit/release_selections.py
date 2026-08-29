"""Reject hardcoded release channels and profile selections.

This guard intentionally uses only the Python standard library. It runs before
Capsem's expensive test stages and therefore must work in the same clean Linux
release environment without assuming developer tools such as ripgrep.
"""

from __future__ import annotations

import os
import re
import sys
from collections.abc import Iterable
from pathlib import Path

from capsem_builder.gate import project_root
from capsem_builder.gate.tools.audit.release_selection_contracts import (
    _builtin_profiles_match,
    _release_workflows_are_serialized,
    _retired_doctrine_reintroduced,
)

PROFILE_TERMS = r"(?:code|co-work|cowork|terminal|termional|gui)"
MATCH_GUARDS = (
    (
        "user-facing session request hardcodes a named profile",
        rf"profile_id\s*:\s*['\"]{PROFILE_TERMS}['\"]",
        ("web/app/src/lib/components", "crates/capsem-tray/src"),
    ),
    (
        "profile picker fabricates a named profile instead of using the installed catalog",
        rf"(?:profileId\s*=[^\n]*['\"]{PROFILE_TERMS}['\"]|"
        rf"<option[^>]*value=['\"]{PROFILE_TERMS}['\"])",
        ("web/app/src/lib/components",),
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
        (
            "build_system/packaging/linux/deb-postinst.sh",
            "build_system/packaging/macos/pkg-scripts/postinstall",
        ),
    ),
    (
        "native postinstall bypasses installed manifest-metadata provenance",
        r"CAPSEM_RELEASE_(?:MANIFEST|HEALTH)_URL",
        (
            "build_system/packaging/linux/deb-postinst.sh",
            "build_system/packaging/macos/pkg-scripts/postinstall",
        ),
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
            "build_system/packaging/macos/build-pkg.sh",
            "build_system/packaging/linux/repack-deb.sh",
            "build_system/packaging/linux/deb-postinst.sh",
            "build_system/packaging/macos/pkg-scripts/postinstall",
            "crates/capsem/src/update.rs",
            "crates/capsem-service/src/main.rs",
        ),
    ),
    (
        "public release HTTP reader passes a bare URL to urllib and may be rejected by the edge",
        r"urlopen\(\s*(?:source|url|manifest_url)\s*,",
        (
            "scripts/materialize-config.sh",
            "build_system/builder/release/tools/build_complete_release_channel.py",
            "build_system/builder/release/tools/local_release_glowup.py",
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

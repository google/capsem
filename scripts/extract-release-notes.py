#!/usr/bin/env python3
"""Extract latest release notes from CHANGELOG.md into LATEST_RELEASE.md.

Called by `just cut-release` after stamping the changelog.
Format: "version: X.Y.Z\n---\n<markdown body>\n"
"""
import re
import sys
from pathlib import Path

changelog = Path("CHANGELOG.md").read_text()

# Find all versioned sections (skip [Unreleased])
matches = list(re.finditer(r"^## \[(\d+\.\d+\.\d+)\]", changelog, re.MULTILINE))
if not matches:
    sys.exit("No versioned release found in CHANGELOG.md")

start = matches[0].end()
end = matches[1].start() if len(matches) > 1 else len(changelog)
body = changelog[start:end].strip()

# Strip the " - YYYY-MM-DD" suffix from the heading line
body = re.sub(r"^[\s-]*\d{4}-\d{2}-\d{2}\s*", "", body).strip()
version = matches[0].group(1)

Path("LATEST_RELEASE.md").write_text(f"version: {version}\n---\n{body}\n")
print(f"LATEST_RELEASE.md updated for v{version}")

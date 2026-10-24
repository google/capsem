"""Immutable upstream documents describe upstream paths, not Capsem's layout."""

import json
from pathlib import Path

RUST_VENDOR_ROOT = "third_party/rust"


def pinned_rust_vendor_documents(root: Path) -> dict[str, str]:
    pinned = {}
    for manifest in (root / RUST_VENDOR_ROOT).glob("*/UPSTREAM.json"):
        if manifest.parent.is_symlink() or manifest.is_symlink():
            continue
        for name, digest in json.loads(manifest.read_text(encoding="utf-8"))["files"].items():
            path = Path(name)
            if path.suffix == ".md" and not path.is_absolute() and ".." not in path.parts:
                pinned[(manifest.parent / path).relative_to(root).as_posix()] = digest
    return pinned

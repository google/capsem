"""One semantic validator for publishable exported-rootfs OBOM evidence."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, cast


def validate_exported_rootfs_obom(path: Path, *, architecture: str | None = None) -> None:
    """Refuse scanner output that was not normalized as guest-rootfs evidence."""
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"cdxgen wrote invalid JSON OBOM at {path}: {exc}") from exc
    if not isinstance(document, dict):
        raise RuntimeError(f"OBOM {path} must be a JSON object")
    if document.get("bomFormat") != "CycloneDX":
        raise RuntimeError(f"OBOM {path} must be CycloneDX JSON")
    metadata = document.get("metadata")
    if not isinstance(metadata, dict):
        raise RuntimeError(f"OBOM {path} is missing metadata")
    tools = metadata.get("tools")
    candidates: list[dict[str, Any]] = []
    if isinstance(tools, dict) and isinstance(tools.get("components"), list):
        candidates = [tool for tool in tools["components"] if isinstance(tool, dict)]
    elif isinstance(tools, list):
        candidates = [tool for tool in tools if isinstance(tool, dict)]
    if not any(
        str(tool.get("name", "")).lower() == "cdxgen" and str(tool.get("version", ""))
        for tool in candidates
    ):
        raise RuntimeError(f"OBOM {path} must record cdxgen name and version in metadata.tools")

    component = metadata.get("component")
    if not isinstance(component, dict):
        raise RuntimeError(f"OBOM {path} is missing metadata.component")
    properties = component.get("properties")
    if not isinstance(properties, list) or not _property(
        properties, "capsem:evidence:scope", "exported-rootfs"
    ):
        raise RuntimeError(f"OBOM {path} is not scoped to the exported rootfs")
    if architecture is not None and (
        component.get("type") != "operating-system"
        or component.get("name") != f"capsem-rootfs-{architecture}"
        or component.get("version") != "guest-rootfs"
        or not _property(properties, "capsem:guest:architecture", architecture)
    ):
        raise RuntimeError(f"OBOM {path} is not normalized for guest architecture {architecture}")

    components = document.get("components")
    if not isinstance(components, list) or not components:
        raise RuntimeError(f"OBOM {path} must inventory guest rootfs components")
    if not any(
        isinstance(item, dict) and str(item.get("purl", "")).startswith("pkg:deb/debian/")
        for item in components
    ):
        raise RuntimeError(f"OBOM {path} does not contain Debian guest packages")
    for item in components:
        if not isinstance(item, dict):
            continue
        item_properties = item.get("properties")
        if isinstance(item_properties, list) and any(
            isinstance(prop, dict) and prop.get("name") == "cdx:osquery:category"
            for prop in item_properties
        ):
            raise RuntimeError(f"OBOM {path} contains live-host inventory")


def _property(properties: list[object], name: str, value: str) -> bool:
    for prop in properties:
        if not isinstance(prop, dict):
            continue
        candidate = cast(dict[object, object], prop)
        if candidate.get("name") == name and candidate.get("value") == value:
            return True
    return False

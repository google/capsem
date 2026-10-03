"""The official image catalog: what this run published, appended to what came before.

Each build leg of the publication workflow leaves a record: one per image and
architecture (its manifest, OBOM and EROFS digests) and one per image (its
multi-architecture index). This turns the records of the images that were
rebuilt into catalog versions and appends them to the previous catalog.

Appending is the contract. A runtime may pin any digest a catalog ever
listed, so a version is never dropped or rewritten -- a rebuild of the same
inputs is a new version beside the old one, not a replacement.

Usage: catalog.py --channel C --registry R --records DIR [--previous FILE] --out FILE
Prints `true` when the catalog gained a version, `false` when it did not.
"""

from __future__ import annotations

import argparse
import copy
import datetime
import json
import re
import tomllib
from collections.abc import Iterable, Mapping, Sequence
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
DESCRIPTIONS = ROOT / "images" / "catalog.toml"
SCHEMA_VERSION = 1
#: The runtime contract every image built here satisfies: user `capsem`
#: (uid 1000), the workspace at /workspace, and the agent on PATH.
CONTRACT = 1
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")


class CatalogError(ValueError):
    """The records or the previous catalog cannot be published as given."""


def descriptions(path: Path = DESCRIPTIONS) -> dict[str, str]:
    images = tomllib.loads(path.read_text(encoding="utf-8"))["images"]
    return {name: entry["description"] for name, entry in images.items()}


def _digest(record: Mapping[str, Any], field: str) -> str:
    value = record.get(field)
    if not isinstance(value, str) or not DIGEST.match(value):
        raise CatalogError(f"record {dict(record)} has no sha256 {field} digest")
    return value


def versions(records: Iterable[Mapping[str, Any]], registry: str) -> dict[str, dict[str, Any]]:
    """One catalog version per rebuilt image, from its index and platform records."""
    indexes: dict[str, Mapping[str, Any]] = {}
    platforms: dict[str, list[Mapping[str, Any]]] = {}
    for record in records:
        if record.get("kind") == "index":
            if record["image"] in indexes:
                raise CatalogError(f"{record['image']} has two index records")
            indexes[record["image"]] = record
        elif record.get("kind") == "platform":
            platforms.setdefault(record["image"], []).append(record)
        else:
            raise CatalogError(f"record {dict(record)} is neither an index nor a platform")
    found: dict[str, dict[str, Any]] = {}
    for name, index in sorted(indexes.items()):
        legs = platforms.get(name, [])
        if not legs or {leg["key"] for leg in legs} != {index["key"]}:
            raise CatalogError(f"{name}: platform records do not match index key {index['key']}")
        if {bool(leg["built"]) for leg in legs} != {bool(index["built"])}:
            raise CatalogError(f"{name}: an architecture was rebuilt without its index")
        if not index["built"]:
            continue
        arches = sorted(leg["arch"] for leg in legs)
        if len(set(arches)) != len(arches):
            raise CatalogError(f"{name}: an architecture has two records")
        found[name] = {
            "image": f"{registry}/{name}@{_digest(index, 'digest')}",
            "platforms": [f"linux/{arch}" for arch in arches],
            "contract": CONTRACT,
            "obom": {leg["arch"]: _digest(leg, "obom") for leg in legs},
            "erofs": {leg["arch"]: _digest(leg, "erofs") for leg in legs},
        }
    orphans = sorted(set(platforms) - set(indexes))
    if orphans:
        raise CatalogError(f"platform records without an index: {', '.join(orphans)}")
    return found


def merge(
    previous: Mapping[str, Any] | None,
    *,
    channel: str,
    generated_at: str,
    described: Mapping[str, str],
    new: Mapping[str, Mapping[str, Any]],
) -> tuple[dict[str, Any], bool]:
    """The previous catalog with `new` appended; never drops or rewrites a version."""
    entries: dict[str, Any] = {}
    if previous is not None:
        if previous.get("schema_version") != SCHEMA_VERSION:
            raise CatalogError(f"previous catalog is not schema {SCHEMA_VERSION}")
        if previous.get("channel") != channel:
            raise CatalogError(
                f"previous catalog is channel {previous.get('channel')!r}, not {channel!r}"
            )
        entries = copy.deepcopy(dict(previous["entries"]))
    changed = False
    for name, version in sorted(new.items()):
        if name not in described:
            raise CatalogError(f"{name} has no description in {DESCRIPTIONS.name}")
        entry = entries.setdefault(name, {"description": described[name], "versions": []})
        if all(old["image"] != version["image"] for old in entry["versions"]):
            entry["versions"].append(dict(version))
            changed = True
    for name, entry in entries.items():
        entry["description"] = described.get(name, entry["description"])
    catalog = {
        "schema_version": SCHEMA_VERSION,
        "channel": channel,
        "generated_at": generated_at,
        "entries": dict(sorted(entries.items())),
    }
    return catalog, changed


def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--channel", required=True, choices=("stable", "nightly"))
    parser.add_argument("--registry", required=True)
    parser.add_argument("--records", required=True, type=Path)
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    records = [json.loads(path.read_text()) for path in sorted(args.records.glob("*.json"))]
    previous = None
    if args.previous is not None:
        previous = json.loads(args.previous.read_text())
    generated_at = datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    catalog, changed = merge(
        previous,
        channel=args.channel,
        generated_at=generated_at,
        described=descriptions(),
        new=versions(records, args.registry),
    )
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(catalog, indent=2, sort_keys=True) + "\n")
    print("true" if changed else "false")


if __name__ == "__main__":
    main()

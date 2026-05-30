#!/usr/bin/env python3
"""Extract exact Preline docs snippets into ignored private/todo files."""

from __future__ import annotations

import argparse
import html
import json
import re
import sys
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "private" / "todo" / "preline"


@dataclass(frozen=True)
class Block:
    component: str
    name: str
    url: str
    textarea_id: str | None = None
    contains: str | None = None


BLOCKS = [
    Block(
        component="alert",
        name="discovery",
        url="https://preline.co/docs/components/alerts.html",
        textarea_id="discovery-tab-html-markup",
    ),
    Block(
        component="modal",
        name="basic",
        url="https://preline.co/docs/components/modal.html",
        contains='id="hs-basic-modal"',
    ),
    Block(
        component="card",
        name="simple",
        url="https://preline.co/docs/components/card.html",
        textarea_id="simple-card-tab-html-markup",
    ),
    Block(
        component="button",
        name="solid-primary",
        url="https://preline.co/docs/components/buttons.html",
        contains="bg-primary border border-primary-line text-primary-foreground",
    ),
    Block(
        component="card",
        name="top-border",
        url="https://preline.co/docs/components/card.html",
        contains="border-t-2 border-primary",
    ),
]


TEXTAREA_RE = re.compile(
    r"<textarea\b(?P<attrs>[^>]*)>(?P<body>.*?)</textarea>", re.IGNORECASE | re.DOTALL
)
ID_RE = re.compile(r"\bid=[\"'](?P<id>[^\"']+)[\"']", re.IGNORECASE)


def fetch(url: str) -> str:
    request = urllib.request.Request(url, headers={"User-Agent": "capsem-ui-scraper/0.1"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode("utf-8")


def textareas(document: str) -> dict[str, str]:
    found: dict[str, str] = {}
    for match in TEXTAREA_RE.finditer(document):
        id_match = ID_RE.search(match.group("attrs"))
        if not id_match:
            continue
        found[id_match.group("id")] = html.unescape(match.group("body")).strip()
    return found


def extract(block: Block, page_textareas: dict[str, str]) -> tuple[str | None, str | None]:
    if block.textarea_id:
        return page_textareas.get(block.textarea_id), block.textarea_id
    if block.contains:
        for textarea_id, snippet in page_textareas.items():
            if block.contains in snippet:
                return snippet, textarea_id
    return None, None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, default=OUT_DIR)
    parser.add_argument("--list", action="store_true", help="List configured blocks.")
    args = parser.parse_args()

    if args.list:
        for block in BLOCKS:
            selector = block.textarea_id or f"contains:{block.contains}"
            print(f"{block.component}/{block.name}: {selector} <- {block.url}")
        return 0

    pages: dict[str, dict[str, str]] = {}
    manifest = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "blocks": [],
    }

    for block in BLOCKS:
        if block.url not in pages:
            pages[block.url] = textareas(fetch(block.url))

        snippet, textarea_id = extract(block, pages[block.url])
        status = "ok" if snippet else "missing"
        output_path = args.out_dir / block.component / block.name / "source.html"

        if snippet:
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_text(snippet + "\n", encoding="utf-8")

        manifest["blocks"].append(
            {
                "component": block.component,
                "name": block.name,
                "source_url": block.url,
                "textarea_id": textarea_id,
                "contains": block.contains,
                "status": status,
                "output_path": str(output_path.relative_to(ROOT)),
            }
        )
        print(f"{status}: {block.component}/{block.name}")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "_manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

"""The development WebView must never load another process's loopback page."""

import json
import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
TAURI = ROOT / "crates/capsem-app/tauri.conf.json"
ASTRO = ROOT / "web/app/astro.config.mjs"
DEV_ORIGIN_RATIONALE = (
    "Tauri grants the development page gateway-token IPC. If Astro silently "
    "moves to another port or binds a different loopback address, Tauri may "
    "load an unrelated local process at its configured origin."
)


def _same_strict_origin(tauri: dict, astro: str) -> bool:
    dev_url = tauri["build"]["devUrl"]
    match = re.search(r"server:\s*\{\s*host:\s*'([^']+)',\s*port:\s*(\d+)\s*\}", astro)
    if match is None:
        return False
    host, port = match.groups()
    return (
        dev_url == f"http://{host}:{port}"
        and host == "127.0.0.1"
        and re.search(r"server:\s*\{\s*strictPort:\s*true\s*\}", astro) is not None
    )


def test_tauri_dev_origin_is_bound_to_astros_strict_loopback_port() -> None:
    assert _same_strict_origin(json.loads(TAURI.read_text()), ASTRO.read_text()), DEV_ORIGIN_RATIONALE


@pytest.mark.parametrize(
    ("dev_url", "host", "strict_port"),
    [
        ("http://localhost:5173", "127.0.0.1", True),
        ("http://127.0.0.1:5174", "127.0.0.1", True),
        ("http://127.0.0.1:5173", "0.0.0.0", True),
        ("http://127.0.0.1:5173", "127.0.0.1", False),
    ],
)
def test_tauri_dev_origin_guard_rejects_mismatches(
    dev_url: str, host: str, strict_port: bool
) -> None:
    tauri = {"build": {"devUrl": dev_url}}
    astro = (
        f"server: {{ host: '{host}', port: 5173 }}, "
        f"vite: {{ server: {{ strictPort: {str(strict_port).lower()} }} }}"
    )
    assert not _same_strict_origin(tauri, astro), DEV_ORIGIN_RATIONALE

"""Public marketing install discoverability contract."""

from __future__ import annotations

from pathlib import Path

PUBLIC_INSTALL_SCRIPT_URL = "https://capsem.org/install.sh"


def validate_marketing_install_surface(
    surface: str,
    *,
    install_script_url: str = PUBLIC_INSTALL_SCRIPT_URL,
) -> None:
    required = (
        f"curl -fsSL {install_script_url} | sh",
        "<Hero />",
        "<CTA",
    )
    if any(token not in surface for token in required) or "Available Summer 2026" in surface:
        raise SystemExit("marketing site does not expose the supported install command")


def validate_checked_in_marketing_install_surface(project_root: Path) -> None:
    sources = (
        project_root / "site/src/pages/index.astro",
        project_root / "site/src/lib/data.ts",
    )
    validate_marketing_install_surface(
        "\n".join(path.read_text(encoding="utf-8") for path in sources)
    )

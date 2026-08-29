"""Release-owned public marketing install discoverability contract."""

from __future__ import annotations

import sys
from pathlib import Path

PUBLIC_INSTALL_SCRIPT_URL = "https://capsem.org/install.sh"
STABLE_PACKAGE_URL = "https://release.capsem.org/channels/stable/"


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


def validate_rendered_marketing_install_surface(
    surface: str,
    *,
    install_script_url: str = PUBLIC_INSTALL_SCRIPT_URL,
) -> None:
    required = (
        f"curl -fsSL {install_script_url} | sh",
        STABLE_PACKAGE_URL,
        "Download Package",
    )
    if any(token not in surface for token in required) or "Available Summer 2026" in surface:
        raise SystemExit("marketing site does not expose the supported install command")


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 1:
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} <rendered-homepage>")
    validate_rendered_marketing_install_surface(
        Path(args[0]).read_text(encoding="utf-8")
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

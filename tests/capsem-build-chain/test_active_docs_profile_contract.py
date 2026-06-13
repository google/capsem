"""Active docs and skills must teach the profile-derived build contract."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]

ACTIVE_DOCS_AND_SKILLS = [
    PROJECT_ROOT / "docs/src/content/docs/architecture/asset-pipeline.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/build-system.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/custom-images.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/mcp-gateway.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/settings-schema.md",
    PROJECT_ROOT / "docs/src/content/docs/development/custom-images.md",
    PROJECT_ROOT / "docs/src/content/docs/development/getting-started.md",
    PROJECT_ROOT / "docs/src/content/docs/development/just-recipes.md",
    PROJECT_ROOT / "docs/src/content/docs/development/stack.md",
    PROJECT_ROOT / "docs/src/content/docs/security/plugins/credential-broker.md",
    PROJECT_ROOT / "skills/build-images/SKILL.md",
    PROJECT_ROOT / "skills/build-initrd/SKILL.md",
    PROJECT_ROOT / "skills/dev-just/SKILL.md",
    PROJECT_ROOT / "skills/dev-testing-frontend/SKILL.md",
    PROJECT_ROOT / "skills/dev-testing-python/SKILL.md",
]

STALE_GUIDANCE = [
    "edit `guest/config",
    "editing `guest/config",
    "TOML configs in `guest/config",
    "All config lives under `guest/config",
    "MCP server definitions live in TOML files under `guest/config/mcp",
    "uv run capsem-builder build guest/",
    "capsem-builder build guest/",
    "capsem-builder init",
    "capsem-builder new",
    "capsem-builder add",
    "capsem-builder add ai-provider",
    "config/admin",
    "settings-registry",
    "settings-schema.generated",
    "mcp-tools.generated",
    "capsem-admin profile init",
    "capsem-admin settings init",
    "capsem-admin manifest verify",
    "capsem-admin image plan",
    "capsem-admin image workspace",
    "capsem-admin image verify",
    "capsem-admin enforcement compile",
    "capsem-admin detection compile",
    "AI providers declare how their CLI gets installed",
    "providers are allowed out of the box",
    "rootfs.squashfs",
]


def test_active_docs_do_not_teach_retired_guest_config_authority() -> None:
    failures: list[str] = []
    for path in ACTIVE_DOCS_AND_SKILLS:
        text = path.read_text()
        for needle in STALE_GUIDANCE:
            if needle in text:
                failures.append(f"{path.relative_to(PROJECT_ROOT)} contains {needle!r}")

    assert not failures, "stale active docs/skills:\n" + "\n".join(sorted(failures))

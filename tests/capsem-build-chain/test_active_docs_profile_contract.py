"""Active docs and skills must teach the profile-derived build contract."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG_ROOT = PROJECT_ROOT / "config"

ALLOWED_CONFIG_DIRS = {
    "corp",
    "data",
    "docker",
    "profiles",
    "settings",
}

FORBIDDEN_CONFIG_DIRS = {
    "admin",
    "default",
    "defaults",
    "guest",
    "preset",
    "presets",
    "registry",
    "schemas",
    "templates",
}

ACTIVE_DOCS_AND_SKILLS = [
    PROJECT_ROOT / "docs/src/content/docs/architecture/asset-pipeline.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/build-system.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/custom-images.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/mcp-gateway.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/service-architecture.md",
    PROJECT_ROOT / "docs/src/content/docs/architecture/settings-schema.md",
    PROJECT_ROOT / "docs/src/content/docs/development/ci.md",
    PROJECT_ROOT / "docs/src/content/docs/development/custom-images.md",
    PROJECT_ROOT / "docs/src/content/docs/development/getting-started.md",
    PROJECT_ROOT / "docs/src/content/docs/development/just-recipes.md",
    PROJECT_ROOT / "docs/src/content/docs/development/stack.md",
    PROJECT_ROOT / "docs/src/content/docs/security/build-verification.md",
    PROJECT_ROOT / "docs/src/content/docs/security/plugins/credential-broker.md",
    PROJECT_ROOT / "skills/asset-pipeline/SKILL.md",
    PROJECT_ROOT / "skills/build-images/SKILL.md",
    PROJECT_ROOT / "skills/build-initrd/SKILL.md",
    PROJECT_ROOT / "skills/dev-capsem/SKILL.md",
    PROJECT_ROOT / "skills/dev-just/SKILL.md",
    PROJECT_ROOT / "skills/dev-skills/SKILL.md",
    PROJECT_ROOT / "skills/dev-sprint/SKILL.md",
    PROJECT_ROOT / "skills/dev-testing-frontend/SKILL.md",
    PROJECT_ROOT / "skills/dev-testing-python/SKILL.md",
]

STALE_GUIDANCE = [
    "edit `guest`/`config",
    "editing `guest`/`config",
    "TOML configs in `guest`/`config",
    "All config lives under `guest`/`config",
    "MCP server definitions live in TOML files under `guest`/`config`/`mcp",
    "uv run capsem-builder build guest/",
    "capsem-builder build guest/",
    "capsem-builder init",
    "capsem-builder new",
    "capsem-builder add",
    "capsem-builder add ai-provider",
    "capsem-builder mcp",
    "config/admin",
    "config/guest",
    "settings registry",
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
    "DMG (codesigned",
    "DMG/deb",
    "capsem-{version}-{arch}.dmg",
    "deb + AppImage",
    "Merges latest.json",
    "latest.json -- Tauri auto-updater metadata",
    "signs manifest",
    "v{VERSION}/{vmlinuz, initrd.img, rootfs.erofs}",
    "hash-pinned sibling",
    "pinned sibling files",
    "BLAKE3/size pins",
    "file pins",
    "payload pins",
    "admin pin",
    "profile payload pins",
    "Refresh payload pins",
    "resolved pins",
    "source pins",
]

RELEASE_ASSET_DOCS = [
    PROJECT_ROOT / "docs/src/content/docs/architecture/asset-pipeline.md",
    PROJECT_ROOT / "docs/src/content/docs/development/ci.md",
    PROJECT_ROOT / "docs/src/content/docs/development/stack.md",
    PROJECT_ROOT / "docs/src/content/docs/security/build-verification.md",
]

BENCHMARK_RESULTS_DOC = PROJECT_ROOT / "docs/src/content/docs/benchmarks/results.md"


def test_active_docs_do_not_teach_retired_guest_config_authority() -> None:
    failures: list[str] = []
    for path in ACTIVE_DOCS_AND_SKILLS:
        text = path.read_text()
        for needle in STALE_GUIDANCE:
            if needle in text:
                failures.append(f"{path.relative_to(PROJECT_ROOT)} contains {needle!r}")

    assert not failures, "stale active docs/skills:\n" + "\n".join(sorted(failures))


def test_release_asset_docs_teach_thin_packages_and_release_assets() -> None:
    required = [
        "arch-prefixed",
        "manifest",
        "rootfs.erofs",
        "verified",
    ]

    failures: list[str] = []
    for path in RELEASE_ASSET_DOCS:
        text = path.read_text()
        for needle in required:
            if needle not in text:
                failures.append(f"{path.relative_to(PROJECT_ROOT)} missing {needle!r}")

    ci_text = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    stack_text = (PROJECT_ROOT / "docs/src/content/docs/development/stack.md").read_text()
    asset_text = (PROJECT_ROOT / "docs/src/content/docs/architecture/asset-pipeline.md").read_text()
    security_text = (PROJECT_ROOT / "docs/src/content/docs/security/build-verification.md").read_text()

    assert "Installers carry host binaries and the selected manifest" in ci_text
    assert "VM assets are not bundled into the installers" in stack_text
    assert "Release installers are intentionally thin" in asset_text
    assert "`arm64-rootfs.erofs`; inside the manifest they remain bare names" in security_text
    assert not failures, "release asset docs missing required wording:\n" + "\n".join(failures)


def test_benchmark_results_page_is_graph_dashboard() -> None:
    text = BENCHMARK_RESULTS_DOC.read_text()
    headings = [
        line.removeprefix("## ").strip()
        for line in text.splitlines()
        if line.startswith("## ")
    ]

    assert headings == ["VM lifecycle", "Disk", "App", "Network"]
    assert text.count("```mermaid") >= 8
    assert text.count("xychart-beta") >= 8

    retired_sections = [
        "Rootfs Decision",
        "Mac DAX Probe",
        "Reproducing",
        "Discussion",
        "Local Network And Model Fixtures",
        "DNS Load",
        "MCP Load",
    ]
    for section in retired_sections:
        assert f"## {section}" not in text


def test_config_root_has_only_declared_authority_directories() -> None:
    actual_dirs = {
        path.name
        for path in CONFIG_ROOT.iterdir()
        if path.is_dir() and not path.name.startswith(".")
    }
    assert actual_dirs == ALLOWED_CONFIG_DIRS

    unexpected_files = [
        path.name
        for path in CONFIG_ROOT.iterdir()
        if path.is_file() and path.name != "README.md" and not path.name.startswith(".")
    ]
    assert unexpected_files == []

    forbidden_present = sorted(FORBIDDEN_CONFIG_DIRS & actual_dirs)
    assert forbidden_present == []


def test_config_readme_declares_authority_and_public_admin_surface() -> None:
    text = (CONFIG_ROOT / "README.md").read_text()
    for directory in sorted(ALLOWED_CONFIG_DIRS):
        assert f"`{directory}/`" in text
    for directory in sorted(FORBIDDEN_CONFIG_DIRS):
        assert f"`{directory}/`" in text

    assert "Settings have a schema; profiles may\nhave a catalog" in text
    assert "Settings do not have a registry" in text
    assert "`profile validate|check|materialize`" in text
    assert "`image build`" in text
    assert "Do not add\n`init`, `new`, `add`" in text

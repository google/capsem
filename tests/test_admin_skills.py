from __future__ import annotations

import re
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
SKILLS_ROOT = PROJECT_ROOT / "skills"
DEV_SKILLS_DOC = (
    PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "development" / "skills.md"
)

ADMIN_SKILLS = {
    "admin-profile": ["profile validate", "editable", "Profile V2"],
    "admin-settings": ["settings validate", "ServiceSettingsV2", "TOML"],
    "admin-image": ["image verify", "image sbom", "doctor-bundle"],
    "admin-manifest": ["manifest check", "minisign", "revoked"],
    "admin-security": ["detection compile", "pySigma", "capsem.detection.ir.v1"],
}

AGENT_SKILL_DIRS = [".claude", ".agents", ".gemini", ".codex", ".cursor"]


def _frontmatter_field(text: str, field: str) -> str:
    match = re.search(rf"^{field}:\s*(.+)$", text, flags=re.MULTILINE)
    assert match, f"missing {field} frontmatter"
    return match.group(1).strip()


def test_admin_skills_are_discoverable_and_specific() -> None:
    for name, required_terms in ADMIN_SKILLS.items():
        skill_path = SKILLS_ROOT / name / "SKILL.md"
        assert skill_path.exists(), f"{name} must be a flat skills/<name>/SKILL.md"

        text = skill_path.read_text(encoding="utf-8")
        assert _frontmatter_field(text, "name") == name
        description = _frontmatter_field(text, "description")
        assert "Use this whenever" in description
        assert "## Testing Checklist" in text

        for term in required_terms:
            assert term in text


def test_admin_skills_docs_reference_agent_clients_and_admin_bundle() -> None:
    doc = DEV_SKILLS_DOC.read_text(encoding="utf-8")
    normalized_doc = " ".join(doc.split())

    for agent_dir in AGENT_SKILL_DIRS:
        assert f"{agent_dir}/skills -> ../skills" in doc
    for skill in ADMIN_SKILLS:
        assert f"`{skill}`" in doc
    assert "bootstrap.sh" in doc
    assert "Claude Code, Gemini CLI, Codex, and Cursor" in normalized_doc


def test_bootstrap_creates_non_destructive_skill_symlinks_for_agent_clients() -> None:
    bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text(encoding="utf-8")

    assert "install_agent_skill_links" in bootstrap
    assert '[ -e "$skill_link" ] && [ ! -L "$skill_link" ]' in bootstrap
    assert 'ln -s ../skills "$skill_link"' in bootstrap
    assert bootstrap.index("install_agent_skill_links") < bootstrap.index(
        "uv run capsem-admin --version"
    )

    for agent_dir in AGENT_SKILL_DIRS:
        assert agent_dir in bootstrap


def test_tracked_agent_skill_symlinks_point_to_shared_skills() -> None:
    for agent_dir in AGENT_SKILL_DIRS:
        link = PROJECT_ROOT / agent_dir / "skills"
        assert link.is_symlink()
        assert link.readlink() == Path("../skills")

"""Tests for Pydantic-backed Capsem skill validation."""

from __future__ import annotations

from pathlib import Path

from click.testing import CliRunner
import pytest

from capsem.builder.cli import cli
from capsem.builder.skills import parse_skill_document, validate_skill_library

PROJECT_ROOT = Path(__file__).parent.parent


def _write_skill(root: Path, name: str, *, frontmatter_name: str | None = None) -> Path:
    skill_dir = root / name
    skill_dir.mkdir(parents=True)
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text(
        "\n".join(
            [
                "---",
                f"name: {frontmatter_name or name}",
                "description: Use when validating the test skill contract with enough detail.",
                "---",
                "",
                "# Test Skill",
                "",
                "Do the thing.",
                "",
            ]
        ),
        encoding="utf-8",
    )
    return skill_path


def test_checked_in_config_skills_validate() -> None:
    report = validate_skill_library(PROJECT_ROOT / "skills")

    assert report.skill_count >= 20
    assert "dev-sprint" in report.skill_names
    assert "build-images" in report.skill_names


def test_skill_frontmatter_name_must_match_directory(tmp_path: Path) -> None:
    skill_path = _write_skill(tmp_path, "dev-real", frontmatter_name="dev-drift")

    with pytest.raises(ValueError, match="must match directory"):
        parse_skill_document(skill_path)


def test_skill_frontmatter_is_required(tmp_path: Path) -> None:
    skill_dir = tmp_path / "dev-bad"
    skill_dir.mkdir()
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text("# Missing frontmatter\n", encoding="utf-8")

    with pytest.raises(ValueError, match="must start with frontmatter"):
        parse_skill_document(skill_path)


def test_skill_library_rejects_symlinked_skill_directory(tmp_path: Path) -> None:
    real_root = tmp_path / "real"
    real_root.mkdir()
    _write_skill(real_root, "dev-real")
    (tmp_path / "dev-link").symlink_to(real_root / "dev-real", target_is_directory=True)

    with pytest.raises(ValueError, match="must not be a symlink"):
        validate_skill_library(tmp_path)


def test_skill_library_rejects_nested_skill_files(tmp_path: Path) -> None:
    _write_skill(tmp_path, "dev-real")
    nested = tmp_path / "dev-real/references/bad"
    nested.mkdir(parents=True)
    (nested / "SKILL.md").write_text(
        "---\nname: nested-bad\ndescription: This nested skill should fail validation.\n---\n# Bad\n",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="nested SKILL.md"):
        validate_skill_library(tmp_path)


def test_validate_skills_cli_accepts_checked_in_skills() -> None:
    result = CliRunner().invoke(cli, ["validate-skills", str(PROJECT_ROOT / "skills")])

    assert result.exit_code == 0, result.output
    assert "skills validated" in result.output


def test_validate_skills_cli_rejects_bad_skills(tmp_path: Path) -> None:
    _write_skill(tmp_path, "dev-real", frontmatter_name="dev-drift")

    result = CliRunner().invoke(cli, ["validate-skills", str(tmp_path)])

    assert result.exit_code == 1
    assert "must match directory" in result.output

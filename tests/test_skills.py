"""Tests for Pydantic-backed Capsem skill validation."""

from __future__ import annotations

from pathlib import Path

from click.testing import CliRunner
import pytest

from capsem.builder.cli import cli
from capsem.builder.skills import parse_skill_document, validate_skill_library

PROJECT_ROOT = Path(__file__).parent.parent


def _write_skill(
    root: Path,
    name: str,
    *,
    frontmatter_name: str | None = None,
    body: str = "# Test Skill\n\nDo the thing.\n",
    description: str = "Use when validating the test skill contract with enough detail.",
) -> Path:
    skill_dir = root / name
    skill_dir.mkdir(parents=True)
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text(
        "\n".join(
            [
                "---",
                f"name: {frontmatter_name or name}",
                f"description: {description}",
                "---",
                "",
                body,
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


def test_skill_frontmatter_name_must_be_kebab_case(tmp_path: Path) -> None:
    skill_path = _write_skill(tmp_path, "dev-real", frontmatter_name="Dev_Real")

    with pytest.raises(ValueError, match="skill name must be lowercase kebab-case"):
        parse_skill_document(skill_path)


def test_skill_directory_name_must_be_kebab_case(tmp_path: Path) -> None:
    skill_path = _write_skill(tmp_path, "DevReal", frontmatter_name="dev-real")

    with pytest.raises(ValueError, match="skill directory must be lowercase kebab-case"):
        parse_skill_document(skill_path)


def test_skill_body_must_not_be_empty(tmp_path: Path) -> None:
    skill_path = _write_skill(tmp_path, "dev-empty", body="   \n")

    with pytest.raises(ValueError, match="skill body must not be empty"):
        parse_skill_document(skill_path)


def test_skill_frontmatter_is_required(tmp_path: Path) -> None:
    skill_dir = tmp_path / "dev-bad"
    skill_dir.mkdir()
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text("# Missing frontmatter\n", encoding="utf-8")

    with pytest.raises(ValueError, match="must start with frontmatter"):
        parse_skill_document(skill_path)


def test_skill_frontmatter_must_be_closed(tmp_path: Path) -> None:
    skill_dir = tmp_path / "dev-bad"
    skill_dir.mkdir()
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text(
        "---\nname: dev-bad\ndescription: Missing the closing marker is invalid.\n",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="frontmatter is not closed"):
        parse_skill_document(skill_path)


def test_skill_frontmatter_rejects_unsupported_lines(tmp_path: Path) -> None:
    skill_dir = tmp_path / "dev-bad"
    skill_dir.mkdir()
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text(
        "---\nname: dev-bad\nnot-a-key-value\ndescription: Unsupported lines fail validation.\n---\nBody\n",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="unsupported frontmatter line"):
        parse_skill_document(skill_path)


def test_skill_frontmatter_rejects_empty_keys(tmp_path: Path) -> None:
    skill_dir = tmp_path / "dev-bad"
    skill_dir.mkdir()
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text(
        "---\nname: dev-bad\n: empty key\ndescription: Empty keys fail validation.\n---\nBody\n",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="frontmatter key must not be empty"):
        parse_skill_document(skill_path)


def test_skill_frontmatter_rejects_duplicate_keys(tmp_path: Path) -> None:
    skill_dir = tmp_path / "dev-bad"
    skill_dir.mkdir()
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text(
        "\n".join(
            [
                "---",
                "name: dev-bad",
                "description: Duplicate keys fail validation.",
                "description: Duplicate keys fail validation again.",
                "---",
                "Body",
            ]
        ),
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="duplicate frontmatter key"):
        parse_skill_document(skill_path)


def test_skill_frontmatter_strips_optional_quotes(tmp_path: Path) -> None:
    skill_dir = tmp_path / "dev-quoted"
    skill_dir.mkdir()
    skill_path = skill_dir / "SKILL.md"
    skill_path.write_text(
        "\n".join(
            [
                "---",
                'name: "dev-quoted"',
                "description: 'Quoted frontmatter values parse as plain strings.'",
                "---",
                "Body",
            ]
        ),
        encoding="utf-8",
    )

    document = parse_skill_document(skill_path)

    assert document.frontmatter.name == "dev-quoted"
    assert document.frontmatter.description == "Quoted frontmatter values parse as plain strings."


def test_skill_document_must_not_be_symlink(tmp_path: Path) -> None:
    real_path = _write_skill(tmp_path, "dev-real")
    link_dir = tmp_path / "dev-link"
    link_dir.mkdir()
    link_path = link_dir / "SKILL.md"
    link_path.symlink_to(real_path)

    with pytest.raises(ValueError, match="SKILL.md must be a real file"):
        parse_skill_document(link_path)


def test_skill_library_rejects_symlinked_skill_directory(tmp_path: Path) -> None:
    real_root = tmp_path / "real"
    real_root.mkdir()
    _write_skill(real_root, "dev-real")
    (tmp_path / "dev-link").symlink_to(real_root / "dev-real", target_is_directory=True)

    with pytest.raises(ValueError, match="must not be a symlink"):
        validate_skill_library(tmp_path)


def test_skill_library_rejects_missing_root(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="skills root does not exist"):
        validate_skill_library(tmp_path / "missing")


def test_skill_library_rejects_file_root(tmp_path: Path) -> None:
    root = tmp_path / "skills.txt"
    root.write_text("not a directory", encoding="utf-8")

    with pytest.raises(ValueError, match="skills root must be a directory"):
        validate_skill_library(root)


def test_skill_library_rejects_symlinked_root(tmp_path: Path) -> None:
    real_root = tmp_path / "real"
    real_root.mkdir()
    _write_skill(real_root, "dev-real")
    link_root = tmp_path / "skills"
    link_root.symlink_to(real_root, target_is_directory=True)

    with pytest.raises(ValueError, match="skills root must be a real directory"):
        validate_skill_library(link_root)


def test_skill_library_ignores_dot_directories(tmp_path: Path) -> None:
    _write_skill(tmp_path, "dev-real")
    hidden = tmp_path / ".cache"
    hidden.mkdir()
    (hidden / "not-a-skill.txt").write_text("ignored", encoding="utf-8")

    report = validate_skill_library(tmp_path)

    assert report.skill_names == ("dev-real",)


def test_skill_library_rejects_file_entries(tmp_path: Path) -> None:
    _write_skill(tmp_path, "dev-real")
    (tmp_path / "README.md").write_text("not a skill directory", encoding="utf-8")

    with pytest.raises(ValueError, match="skills root entries must be directories"):
        validate_skill_library(tmp_path)


def test_skill_library_rejects_missing_skill_document(tmp_path: Path) -> None:
    (tmp_path / "dev-missing").mkdir()

    with pytest.raises(ValueError, match="missing SKILL.md"):
        validate_skill_library(tmp_path)


def test_skill_library_rejects_empty_root(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="must contain at least one skill"):
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

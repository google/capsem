"""Pydantic-backed validation for Capsem skill libraries."""

from __future__ import annotations

import re
from pathlib import Path

from pydantic import BaseModel, ConfigDict, Field, field_validator

SKILL_ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")


class SkillFrontmatter(BaseModel):
    """Validated `SKILL.md` frontmatter."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    name: str = Field(min_length=1, max_length=64)
    description: str = Field(min_length=24, max_length=2048)

    @field_validator("name")
    @classmethod
    def validate_name(cls, value: str) -> str:
        if not SKILL_ID_RE.fullmatch(value):
            msg = "skill name must be lowercase kebab-case, 1-64 chars"
            raise ValueError(msg)
        return value


class SkillDocument(BaseModel):
    """A parsed skill document with its source path."""

    model_config = ConfigDict(frozen=True)

    directory_name: str
    path: Path
    frontmatter: SkillFrontmatter
    body: str

    @field_validator("directory_name")
    @classmethod
    def validate_directory_name(cls, value: str) -> str:
        if not SKILL_ID_RE.fullmatch(value):
            msg = "skill directory must be lowercase kebab-case, 1-64 chars"
            raise ValueError(msg)
        return value

    def validate_contract(self) -> None:
        """Raise `ValueError` when path and frontmatter drift."""
        if self.frontmatter.name != self.directory_name:
            msg = (
                f"frontmatter name {self.frontmatter.name!r} must match "
                f"directory {self.directory_name!r}"
            )
            raise ValueError(msg)
        if not self.body.strip():
            raise ValueError("skill body must not be empty")


class SkillLibraryReport(BaseModel):
    """Summary returned after validating a skills directory."""

    model_config = ConfigDict(frozen=True)

    root: Path
    skill_count: int
    skill_names: tuple[str, ...]


def parse_skill_document(path: Path) -> SkillDocument:
    """Parse and validate one `SKILL.md` file."""
    if path.is_symlink():
        raise ValueError(f"{path}: SKILL.md must be a real file, not a symlink")
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        raise ValueError(f"{path}: SKILL.md must start with frontmatter marker ---")

    end_index = None
    for index, line in enumerate(lines[1:], start=1):
        if line.strip() == "---":
            end_index = index
            break
    if end_index is None:
        raise ValueError(f"{path}: SKILL.md frontmatter is not closed with ---")

    frontmatter = _parse_frontmatter(lines[1:end_index], path)
    document = SkillDocument(
        directory_name=path.parent.name,
        path=path,
        frontmatter=SkillFrontmatter.model_validate(frontmatter),
        body="\n".join(lines[end_index + 1 :]),
    )
    document.validate_contract()
    return document


def validate_skill_library(root: Path) -> SkillLibraryReport:
    """Validate a canonical Capsem skill library directory."""
    if not root.exists():
        raise ValueError(f"{root}: skills root does not exist")
    if not root.is_dir():
        raise ValueError(f"{root}: skills root must be a directory")
    if root.is_symlink():
        raise ValueError(f"{root}: skills root must be a real directory, not a symlink")

    documents: list[SkillDocument] = []
    for child in sorted(root.iterdir(), key=lambda item: item.name):
        if child.name.startswith("."):
            continue
        if not child.is_dir():
            raise ValueError(f"{child}: skills root entries must be directories")
        if child.is_symlink():
            raise ValueError(f"{child}: skill directory must not be a symlink")
        skill_path = child / "SKILL.md"
        if not skill_path.exists():
            raise ValueError(f"{child}: missing SKILL.md")
        documents.append(parse_skill_document(skill_path))

    if not documents:
        raise ValueError(f"{root}: skills root must contain at least one skill")

    nested = [
        path
        for path in root.rglob("SKILL.md")
        if path.parent.parent != root and path.parent != root
    ]
    if nested:
        paths = ", ".join(str(path.relative_to(root)) for path in sorted(nested))
        raise ValueError(f"{root}: nested SKILL.md files are not valid skill roots: {paths}")

    names = tuple(document.frontmatter.name for document in documents)
    if len(set(names)) != len(names):
        raise ValueError(f"{root}: duplicate skill names are not allowed")

    return SkillLibraryReport(root=root, skill_count=len(documents), skill_names=names)


def _parse_frontmatter(lines: list[str], path: Path) -> dict[str, str]:
    parsed: dict[str, str] = {}
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if ":" not in stripped:
            raise ValueError(f"{path}: unsupported frontmatter line {line!r}")
        key, raw_value = stripped.split(":", 1)
        key = key.strip()
        value = raw_value.strip()
        if not key:
            raise ValueError(f"{path}: frontmatter key must not be empty")
        if key in parsed:
            raise ValueError(f"{path}: duplicate frontmatter key {key!r}")
        parsed[key] = _strip_optional_quotes(value)
    return parsed


def _strip_optional_quotes(value: str) -> str:
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
        return value[1:-1]
    return value

"""Compose data supplied by an evaluator; no host discovery or authority."""

from __future__ import annotations

import fnmatch
import logging
import os
import posixpath
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path, PurePath
from types import MappingProxyType
from typing import Any

from .dockerfile_metadata import _extract_dockerfile_metadata

logger = logging.getLogger(__name__)
CAPSEM_INSPECT_ALLOWED_HOST_ENV_VAR = "CAPSEM_INSPECT_ALLOWED_HOST_ENV"
_ENV_PAT_RE = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_*?]*$")
_SAMPLE_METADATA_PREFIX = "SAMPLE_METADATA_"
AllowedEnvSpec = Sequence[str] | tuple[tuple[str, ...], tuple[str, ...]]


def validate_host_env_patterns(patterns: Sequence[str], *, source: str) -> tuple[str, ...]:
    """Validate and strip host-env allowlist patterns."""
    cleaned: list[str] = []
    for raw in patterns:
        if not (pat := raw.strip()):
            continue
        if not _ENV_PAT_RE.match(pat):
            raise ValueError(
                f"Wildcard-only or invalid pattern {pat!r} is not allowed in {source}; "
                "specify explicit variable names or a literal prefix such as 'MY_APP_*'."
            )
        cleaned.append(pat)
    return tuple(cleaned)


_validate_host_env_patterns = validate_host_env_patterns


def resolve_effective_allowed_host_env(
    task_allowed_host_env: Sequence[str] = (),
) -> tuple[str, ...] | tuple[tuple[str, ...], tuple[str, ...]]:
    """Return effective host-env patterns from operator env narrowed by task config."""
    task_pats = validate_host_env_patterns(task_allowed_host_env, source="allowed_host_env")
    op_pats = validate_host_env_patterns(
        os.environ.get(CAPSEM_INSPECT_ALLOWED_HOST_ENV_VAR, "").split(","),
        source=CAPSEM_INSPECT_ALLOWED_HOST_ENV_VAR,
    )
    return ((op_pats, task_pats) if task_pats else op_pats) if op_pats else ()


def _is_host_env_allowed(name: str, allowed: AllowedEnvSpec) -> bool:
    """Return True if `name` matches the effective host-env allowlist."""
    if not allowed:
        return False
    if isinstance(allowed[0], tuple):
        return all(any(fnmatch.fnmatchcase(name, p) for p in grp) for grp in allowed)
    return any(fnmatch.fnmatchcase(name, str(pat)) for pat in allowed)


def _warn_blocked_host_env(name: str, warned_blocked: set[str] | None) -> None:
    if warned_blocked is None or name not in warned_blocked:
        if warned_blocked is not None:
            warned_blocked.add(name)
        logger.warning(
            "Ignoring non-allowlisted host environment variable %r in Compose "
            "(set %s or allowed_host_env to pass it through).",
            name,
            CAPSEM_INSPECT_ALLOWED_HOST_ENV_VAR,
        )


def build_sample_metadata_env(sample_metadata: Mapping[str, Any] | None) -> dict[str, str]:
    """Convert sample `metadata` into `SAMPLE_METADATA_<KEY>` strings."""
    if not sample_metadata:
        return {}
    return {
        f"{_SAMPLE_METADATA_PREFIX}{str(k).replace(' ', '_').upper()}": str(v)
        for k, v in sample_metadata.items()
        if v is not None
    }


@dataclass(frozen=True)
class ComposeInputs:
    environment: Mapping[str, str] = field(default_factory=dict)
    files: Mapping[str, str] = field(default_factory=dict)
    directories: frozenset[str] = frozenset()
    blocked_environment: frozenset[str] = frozenset()
    warned_blocked: set[str] = field(default_factory=set, compare=False, repr=False, hash=False)

    def __post_init__(self) -> None:
        object.__setattr__(self, "environment", MappingProxyType(dict(self.environment)))
        object.__setattr__(
            self,
            "files",
            MappingProxyType({self.resolve(key): value for key, value in self.files.items()}),
        )
        object.__setattr__(
            self, "directories", frozenset(self.resolve(path) for path in self.directories)
        )
        object.__setattr__(self, "blocked_environment", frozenset(self.blocked_environment))

    @staticmethod
    def resolve(path: str | PurePath) -> str:
        """Lexical normalization, without cwd, symlink resolution or stat."""
        return posixpath.normpath(str(path))

    def is_dir(self, path: str | PurePath) -> bool:
        return self.resolve(path) in self.directories

    def exists(self, path: str | PurePath) -> bool:
        key = self.resolve(path)
        return key in self.files or key in self.directories

    def lookup_env(self, key: str) -> str:
        if key in self.environment:
            return self.environment[key]
        if key in self.blocked_environment:
            _warn_blocked_host_env(key, self.warned_blocked)
        return ""

    def dockerfile_defaults(self, path: str | PurePath) -> dict[str, str]:
        text = self.files.get(self.resolve(path))
        if text is None:
            return {}
        workdir, user = _extract_dockerfile_metadata(text)
        return {
            key: value
            for key, value in [("working_dir", workdir), ("user", user)]
            if value is not None
        }


EMPTY_INPUTS = ComposeInputs()


@dataclass(frozen=True)
class ComposeLimits:
    maximum_bytes: int
    maximum_nodes: int
    maximum_depth: int

    def __post_init__(self) -> None:
        if any(
            type(value) is not int or value <= 0
            for value in [self.maximum_bytes, self.maximum_nodes, self.maximum_depth]
        ):
            raise ValueError("Compose limits must be positive integers")


DEFAULT_COMPOSE_LIMITS = ComposeLimits(maximum_bytes=262144, maximum_nodes=4096, maximum_depth=32)


class InterpolationBudget:
    """Bound intermediate expanded fragments as well as the output tree."""

    def __init__(
        self,
        limits: ComposeLimits,
        *,
        blocked_environment: frozenset[str] = frozenset(),
        warned_blocked: set[str] | None = None,
    ):
        self.limits = limits
        self.remaining_bytes = limits.maximum_bytes
        self.remaining_nodes = limits.maximum_nodes
        self.blocked_environment = blocked_environment
        self.warned_blocked = warned_blocked

    def warn_blocked(self, name: str) -> None:
        _warn_blocked_host_env(name, self.warned_blocked)

    def depth(self, depth: int) -> None:
        if depth > self.limits.maximum_depth:
            raise ValueError("Compose depth limit exceeded")

    def node(self, depth: int) -> None:
        self.depth(depth)
        self.remaining_nodes -= 1
        if self.remaining_nodes < 0:
            raise ValueError("Compose node limit exceeded")

    def consume(self, value: str) -> None:
        self.remaining_bytes -= len(value.encode("utf-8"))
        if self.remaining_bytes < 0:
            raise ValueError("Compose expanded byte limit exceeded")


class Fragments(list[str]):
    def __init__(self, budget: InterpolationBudget):
        super().__init__()
        self.budget = budget

    def append(self, value: str) -> None:
        self.budget.consume(value)
        super().append(value)


def build_host_compose_inputs(
    compose_path: Path | None = None,
    *,
    base_dir: Path | None = None,
    allowed_host_env: Sequence[str] = (),
    sample_metadata: Mapping[str, Any] | None = None,
    include_dotenv: bool = False,
    limits: ComposeLimits = DEFAULT_COMPOSE_LIMITS,
) -> ComposeInputs:
    """Build `ComposeInputs` from operator-allowlisted host env, `.env`, and sample metadata."""
    eff_dir = compose_path.parent if compose_path is not None else base_dir
    files: dict[str, str] = {}
    if compose_path is not None and compose_path.is_file():
        files[str(compose_path)] = compose_path.read_text(encoding="utf-8")
    dotenv_text = ""
    if eff_dir is not None and (dotenv_path := eff_dir / ".env").is_file():
        dotenv_text = dotenv_path.read_text(encoding="utf-8")
        files[str(dotenv_path)] = dotenv_text
    eff_allowed = resolve_effective_allowed_host_env(allowed_host_env)
    allowed_env: dict[str, str] = {}
    blocked_keys: set[str] = set()
    for k, v in os.environ.items():
        if k.startswith(_SAMPLE_METADATA_PREFIX):
            continue
        if _is_host_env_allowed(k, eff_allowed):
            allowed_env[k] = v
        else:
            blocked_keys.add(k)
    sample_env = build_sample_metadata_env(sample_metadata)
    dotenv_vars: dict[str, str] = {}
    if include_dotenv and dotenv_text:
        from .compose_interpolation import _load_dotenv

        dotenv_vars = _load_dotenv(
            dotenv_text, {**allowed_env, **sample_env}, InterpolationBudget(limits)
        )
        blocked_keys.difference_update(dotenv_vars)
    return ComposeInputs(
        environment={**dotenv_vars, **allowed_env, **sample_env},
        files=files,
        blocked_environment=frozenset(blocked_keys),
    )

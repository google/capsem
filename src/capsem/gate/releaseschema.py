"""Validated release-authority values from ``config/gate.toml``."""

from __future__ import annotations

from typing import Annotated

from pydantic import StringConstraints, field_validator

from capsem.releasechannel import FirstPartyChannel

from .configschema import Strict


class RetiredPublicGraphConfig(Strict):
    channel: FirstPartyChannel
    sha256: Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{64}$")]


class ReleaseConfig(Strict):
    line: Annotated[str, StringConstraints(pattern=r"^\d+\.\d+$")]
    locally_qualified_channels: tuple[str, ...]
    """Channels releasable only from a commit an operator qualified.

    Nightly is deliberately absent. Spec 13.2 rebuilds current `main` every day
    with nobody involved, and the lanes it dispatches prove themselves through
    `qualify-assets` and `qualify-binaries`. Demanding a machine-local journal
    there made the scheduled rebuild unsatisfiable: a fresh runner has none and
    cannot make one.
    """
    clean_worktree: str
    source: str
    source_ref_template: str
    tagger_name: Annotated[str, StringConstraints(min_length=1, max_length=128)]
    tagger_email: Annotated[str, StringConstraints(pattern=r"^[^@\s]+@[^@\s]+$")]
    notes: tuple[str, ...]
    fetch_manifest: str
    binaries: str
    profile: tuple[str, ...]
    preflight_dir: str
    channel_source: str
    default_repository: str
    repository_variable: str
    token_variable: str
    retired_public_graphs: tuple[RetiredPublicGraphConfig, ...]

    @field_validator("source_ref_template")
    @classmethod
    def _source_ref_is_one_commit_derived_tag(cls, template: str) -> str:
        if template != "capsem-source-{source_commit}":
            raise ValueError("release source_ref_template must be capsem-source-{source_commit}")
        return template

    @field_validator("retired_public_graphs")
    @classmethod
    def _retired_channels_are_unique(
        cls, rows: tuple[RetiredPublicGraphConfig, ...]
    ) -> tuple[RetiredPublicGraphConfig, ...]:
        channels = [row.channel for row in rows]
        if len(channels) != len(set(channels)):
            raise ValueError("release retired_public_graphs channels must be unique")
        return rows

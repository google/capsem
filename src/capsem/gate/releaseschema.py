"""Validated release-authority values from ``config/gate.toml``."""

from __future__ import annotations

from typing import Annotated

from capsem_builder.release.releasechannel import FirstPartyChannel
from pydantic import StringConstraints, field_validator

from .configschema import Strict


class ReleasePairingEnvironment(Strict):
    """What a glow-up is told about the transition it is proving.

    One table because it is one fact: which channel, from which public state to
    which candidate, and where each side's verified cohort is. The glow-up reads
    all seven or none -- it refuses a partial set for the same reason
    `qualification` refuses a half-set release environment, and for the same
    consequence: a partial one still produces a green proof of the wrong thing.
    """

    channel: str
    baseline_channel: str
    transition: str
    before_manifest: str
    after_manifest: str
    before_profile_inputs: str
    after_profile_inputs: str

    def runtime(
        self,
        *,
        channel: object,
        baseline_channel: object,
        transition: object,
        before_manifest: object,
        after_manifest: object,
        before_profile_inputs: object,
        after_profile_inputs: object,
    ) -> dict[str, str]:
        """The exact transition one glow-up run is proving."""
        return {
            self.channel: str(channel),
            self.baseline_channel: str(baseline_channel),
            self.transition: str(transition),
            self.before_manifest: str(before_manifest),
            self.after_manifest: str(after_manifest),
            self.before_profile_inputs: str(before_profile_inputs),
            self.after_profile_inputs: str(after_profile_inputs),
        }

    @property
    def variables(self) -> tuple[str, ...]:
        """Every name this table declares, for whoever has to clear them."""
        return (
            self.channel,
            self.baseline_channel,
            self.transition,
            self.before_manifest,
            self.after_manifest,
            self.before_profile_inputs,
            self.after_profile_inputs,
        )


class RetiredPublicGraphConfig(Strict):
    channel: FirstPartyChannel
    sha256: Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{64}$")]


class ReleaseConfig(Strict):
    line: Annotated[str, StringConstraints(pattern=r"^\d+\.\d+$")]
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

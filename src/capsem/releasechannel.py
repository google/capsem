"""Closed first-party release-channel identities used at trust boundaries."""

from __future__ import annotations

from enum import StrEnum


class FirstPartyChannel(StrEnum):
    """A Capsem-operated public channel; corporate channels are separate."""

    STABLE = "stable"
    NIGHTLY = "nightly"

    @classmethod
    def parse(cls, value: str) -> FirstPartyChannel:
        try:
            return cls(value)
        except ValueError as error:
            raise ValueError("first-party channel must be stable or nightly") from error

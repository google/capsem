"""Finite route evidence is distinct from concurrent read-only polling."""
from typing import Literal

from pydantic import Field

from .configschema import Strict


class FiniteEndpointEvidence(Strict):
    collector: Literal['routes']
    target_scope: Literal['both']
    reason: str = Field(min_length=20)
    cases: dict[str, Literal['cycles', 'cached_samples', 'refresh_samples', 'inventory_samples', 'warm_pulls']] = Field(min_length=1)

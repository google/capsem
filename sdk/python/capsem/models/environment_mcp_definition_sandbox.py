"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictStr

from .environment_secret_binding import EnvironmentSecretBinding
from .model_base import Model


class EnvironmentMcpDefinitionSandbox(Model):
    nonnullable_optional = frozenset(['args', 'secrets', 'variables'])
    args: list[StrictStr] | None = None
    executable: StrictStr
    kind: Literal["sandbox"]
    secrets: dict[str, EnvironmentSecretBinding] | None = None
    variables: dict[str, StrictStr] | None = None
    workdir: StrictStr | None = None

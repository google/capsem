"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import Literal

from pydantic import StrictStr

from .mcp_catalog_requirement import McpCatalogRequirement
from .model_base import Model


class McpCatalogDefinitionSandbox(Model):
    nonnullable_optional = frozenset(['args', 'secret_variables', 'variables'])
    args: list[StrictStr] | None = None
    executable: StrictStr
    kind: Literal["sandbox"]
    secret_variables: dict[str, McpCatalogRequirement] | None = None
    variables: dict[str, McpCatalogRequirement] | None = None
    workdir: StrictStr | None = None

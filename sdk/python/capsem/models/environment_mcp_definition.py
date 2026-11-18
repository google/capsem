"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .environment_mcp_definition_remote import EnvironmentMcpDefinitionRemote
from .environment_mcp_definition_sandbox import EnvironmentMcpDefinitionSandbox

EnvironmentMcpDefinition: TypeAlias = EnvironmentMcpDefinitionRemote | EnvironmentMcpDefinitionSandbox

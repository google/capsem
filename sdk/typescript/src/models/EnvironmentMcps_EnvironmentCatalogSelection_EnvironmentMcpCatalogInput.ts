// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogSelection } from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import type { EnvironmentMcpDefinition } from "./EnvironmentMcpDefinition.js";
import type { EnvironmentMcpPermissions } from "./EnvironmentMcpPermissions.js";
import type { EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue } from "./EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue.js";

export interface EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput {
  "catalog"?: EnvironmentCatalogSelections_EnvironmentCatalogSelection;
  "catalog_configuration"?: Record<string, EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue> | null;
  "permissions"?: Record<string, EnvironmentMcpPermissions>;
  "servers"?: Record<string, EnvironmentMcpDefinition>;
}

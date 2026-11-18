// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogSelection } from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import type { EnvironmentMcpDefinition } from "./EnvironmentMcpDefinition.js";
import type { EnvironmentMcpPermissions } from "./EnvironmentMcpPermissions.js";

export interface EnvironmentMcps_EnvironmentCatalogSelection {
  "catalog"?: EnvironmentCatalogSelections_EnvironmentCatalogSelection;
  "permissions"?: Record<string, EnvironmentMcpPermissions>;
  "servers"?: Record<string, EnvironmentMcpDefinition>;
}

// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogPin } from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import type { EnvironmentMcpDefinition } from "./EnvironmentMcpDefinition.js";
import type { EnvironmentMcpPermissions } from "./EnvironmentMcpPermissions.js";

export interface EnvironmentMcps_EnvironmentCatalogPin {
  "catalog"?: EnvironmentCatalogSelections_EnvironmentCatalogPin;
  "permissions"?: Record<string, EnvironmentMcpPermissions>;
  "servers"?: Record<string, EnvironmentMcpDefinition>;
}

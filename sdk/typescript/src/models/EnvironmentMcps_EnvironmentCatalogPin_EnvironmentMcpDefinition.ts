// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentCatalogSelections_EnvironmentCatalogPin } from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import type { EnvironmentMcpDefinition } from "./EnvironmentMcpDefinition.js";
import type { EnvironmentMcpPermissions } from "./EnvironmentMcpPermissions.js";
import type { EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0 } from "./EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0.js";
import type { EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1 } from "./EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1.js";

export interface EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition {
  "catalog"?: EnvironmentCatalogSelections_EnvironmentCatalogPin;
  "catalog_configuration"?: Record<string, EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0 | EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1> | null;
  "permissions"?: Record<string, EnvironmentMcpPermissions>;
  "servers"?: Record<string, EnvironmentMcpDefinition>;
}

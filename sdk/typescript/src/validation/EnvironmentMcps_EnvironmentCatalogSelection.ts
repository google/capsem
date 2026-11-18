// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcps_EnvironmentCatalogSelection} from "../models/EnvironmentMcps_EnvironmentCatalogSelection.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import {EnvironmentMcpDefinitionSchema} from "./EnvironmentMcpDefinition.js";
import {EnvironmentMcpPermissionsSchema} from "./EnvironmentMcpPermissions.js";

export const EnvironmentMcps_EnvironmentCatalogSelectionSchema: z.ZodType<EnvironmentMcps_EnvironmentCatalogSelection> = z.strictObject({
  "catalog": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema).exactOptional(),
  "permissions": z.record(z.string(), z.lazy(() => EnvironmentMcpPermissionsSchema)).exactOptional(),
  "servers": z.record(z.string(), z.lazy(() => EnvironmentMcpDefinitionSchema)).exactOptional(),
});

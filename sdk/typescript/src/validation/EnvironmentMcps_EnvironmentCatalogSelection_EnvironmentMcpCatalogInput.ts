// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput} from "../models/EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import {EnvironmentMcpDefinitionSchema} from "./EnvironmentMcpDefinition.js";
import {EnvironmentMcpPermissionsSchema} from "./EnvironmentMcpPermissions.js";
import {EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValueSchema} from "./EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue.js";

export const EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputSchema: z.ZodType<EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInput> = z.strictObject({
  "catalog": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema).exactOptional(),
  "catalog_configuration": z.record(z.string(), z.lazy(() => EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValueSchema)).nullable().exactOptional(),
  "permissions": z.record(z.string(), z.lazy(() => EnvironmentMcpPermissionsSchema)).exactOptional(),
  "servers": z.record(z.string(), z.lazy(() => EnvironmentMcpDefinitionSchema)).exactOptional(),
});

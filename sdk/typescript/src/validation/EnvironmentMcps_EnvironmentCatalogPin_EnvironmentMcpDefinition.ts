// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition} from "../models/EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogPinSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import {EnvironmentMcpDefinitionSchema} from "./EnvironmentMcpDefinition.js";
import {EnvironmentMcpPermissionsSchema} from "./EnvironmentMcpPermissions.js";
import {EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0Schema} from "./EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0.js";
import {EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1Schema} from "./EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1.js";

export const EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionSchema: z.ZodType<EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinition> = z.strictObject({
  "catalog": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogPinSchema).exactOptional(),
  "catalog_configuration": z.record(z.string(), z.union([z.lazy(() => EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0Schema), z.lazy(() => EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1Schema)])).nullable().exactOptional(),
  "permissions": z.record(z.string(), z.lazy(() => EnvironmentMcpPermissionsSchema)).exactOptional(),
  "servers": z.record(z.string(), z.lazy(() => EnvironmentMcpDefinitionSchema)).exactOptional(),
});

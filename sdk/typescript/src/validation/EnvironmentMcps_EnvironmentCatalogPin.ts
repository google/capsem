// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcps_EnvironmentCatalogPin} from "../models/EnvironmentMcps_EnvironmentCatalogPin.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogPinSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import {EnvironmentMcpDefinitionSchema} from "./EnvironmentMcpDefinition.js";
import {EnvironmentMcpPermissionsSchema} from "./EnvironmentMcpPermissions.js";

export const EnvironmentMcps_EnvironmentCatalogPinSchema: z.ZodType<EnvironmentMcps_EnvironmentCatalogPin> = z.strictObject({
  "catalog": z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogPinSchema).exactOptional(),
  "permissions": z.record(z.string(), z.lazy(() => EnvironmentMcpPermissionsSchema)).exactOptional(),
  "servers": z.record(z.string(), z.lazy(() => EnvironmentMcpDefinitionSchema)).exactOptional(),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue} from "../models/EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue.js";
import {EnvironmentSecretBindingSchema} from "./EnvironmentSecretBinding.js";

export const EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValueSchema: z.ZodType<EnvironmentMcps_EnvironmentCatalogSelection_EnvironmentMcpCatalogInputValue> = z.strictObject({
  "binding_id": z.string().nullable().exactOptional(),
  "secrets": z.record(z.string(), z.lazy(() => EnvironmentSecretBindingSchema)).exactOptional(),
  "values": z.record(z.string(), z.string()).exactOptional(),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0} from "../models/EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0.js";

export const EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0Schema: z.ZodType<EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative0> = z.object({
  "binding_id": z.string().nullable().exactOptional(),
  "headers": z.record(z.string(), z.string()).exactOptional(),
  "kind": z.literal("remote"),
  "url": z.string(),
});

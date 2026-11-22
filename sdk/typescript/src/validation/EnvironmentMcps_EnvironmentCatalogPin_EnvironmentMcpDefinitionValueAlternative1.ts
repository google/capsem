// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1} from "../models/EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1.js";
import {EnvironmentSecretBindingSchema} from "./EnvironmentSecretBinding.js";

export const EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1Schema: z.ZodType<EnvironmentMcps_EnvironmentCatalogPin_EnvironmentMcpDefinitionValueAlternative1> = z.object({
  "args": z.array(z.string()).exactOptional(),
  "executable": z.string(),
  "kind": z.literal("sandbox"),
  "secrets": z.record(z.string(), z.lazy(() => EnvironmentSecretBindingSchema)).exactOptional(),
  "variables": z.record(z.string(), z.string()).exactOptional(),
  "workdir": z.string().nullable().exactOptional(),
});

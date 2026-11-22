// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpCatalogDefinitionSandbox} from "../models/McpCatalogDefinitionSandbox.js";
import {McpCatalogRequirementSchema} from "./McpCatalogRequirement.js";

export const McpCatalogDefinitionSandboxSchema: z.ZodType<McpCatalogDefinitionSandbox> = z.object({
  "args": z.array(z.string()).exactOptional(),
  "executable": z.string(),
  "kind": z.literal("sandbox"),
  "secret_variables": z.record(z.string(), z.lazy(() => McpCatalogRequirementSchema)).exactOptional(),
  "variables": z.record(z.string(), z.lazy(() => McpCatalogRequirementSchema)).exactOptional(),
  "workdir": z.string().nullable().exactOptional(),
});

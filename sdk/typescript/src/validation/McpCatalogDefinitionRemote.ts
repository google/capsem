// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpCatalogDefinitionRemote} from "../models/McpCatalogDefinitionRemote.js";
import {McpCatalogRequirementSchema} from "./McpCatalogRequirement.js";

export const McpCatalogDefinitionRemoteSchema: z.ZodType<McpCatalogDefinitionRemote> = z.object({
  "headers": z.record(z.string(), z.lazy(() => McpCatalogRequirementSchema)).exactOptional(),
  "kind": z.literal("remote"),
  "secret_binding": z.union([z.null(), z.lazy(() => McpCatalogRequirementSchema)]).exactOptional(),
  "url": z.string(),
});

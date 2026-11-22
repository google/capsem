// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpCatalogRequirement} from "../models/McpCatalogRequirement.js";

export const McpCatalogRequirementSchema: z.ZodType<McpCatalogRequirement> = z.strictObject({
  "description": z.string(),
  "required": z.boolean(),
});

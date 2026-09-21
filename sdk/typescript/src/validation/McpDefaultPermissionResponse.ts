// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpDefaultPermissionResponse} from "../models/McpDefaultPermissionResponse.js";
import {McpPermissionActionSchema} from "./McpPermissionAction.js";

export const McpDefaultPermissionResponseSchema: z.ZodType<McpDefaultPermissionResponse> = z.object({
  "action": z.lazy(() => McpPermissionActionSchema),
  "rule_id": z.string().nullable().exactOptional(),
  "source": z.string(),
});

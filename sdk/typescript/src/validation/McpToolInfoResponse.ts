// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpToolInfoResponse} from "../models/McpToolInfoResponse.js";
import {McpPermissionActionSchema} from "./McpPermissionAction.js";

export const McpToolInfoResponseSchema: z.ZodType<McpToolInfoResponse> = z.object({
  "annotations": z.json().exactOptional(),
  "description": z.string().nullable().exactOptional(),
  "namespaced_name": z.string(),
  "original_name": z.string(),
  "permission_action": z.lazy(() => McpPermissionActionSchema),
  "permission_source": z.string(),
  "pin_changed": z.boolean(),
  "pin_hash": z.string().nullable().exactOptional(),
  "server_name": z.string(),
});

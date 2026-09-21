// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpRefreshResponse} from "../models/McpRefreshResponse.js";

export const McpRefreshResponseSchema: z.ZodType<McpRefreshResponse> = z.object({
  "instances": z.int().min(0),
  "server_id": z.string(),
  "success": z.boolean(),
});

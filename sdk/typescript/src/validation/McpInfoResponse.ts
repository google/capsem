// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpInfoResponse} from "../models/McpInfoResponse.js";

export const McpInfoResponseSchema: z.ZodType<McpInfoResponse> = z.object({
  "builtin_local_enabled": z.boolean(),
  "manual_server_count": z.int().min(0),
  "server_count": z.int().min(0),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpUsage} from "../models/McpUsage.js";

export const McpUsageSchema: z.ZodType<McpUsage> = z.object({
  "bytes_received": z.int().min(0),
  "bytes_sent": z.int().min(0),
  "call_count": z.int().min(0),
  "duration_ms": z.int().min(0),
  "server_name": z.string().nullable().exactOptional(),
  "tool_name": z.string(),
});

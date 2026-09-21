// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpServerInfoResponse} from "../models/McpServerInfoResponse.js";

export const McpServerInfoResponseSchema: z.ZodType<McpServerInfoResponse> = z.object({
  "custom_header_count": z.int().min(0),
  "enabled": z.boolean(),
  "has_auth_credential": z.boolean(),
  "is_stdio": z.boolean(),
  "name": z.string(),
  "running": z.boolean(),
  "source": z.string(),
  "tool_count": z.int().min(0),
  "url": z.string(),
});

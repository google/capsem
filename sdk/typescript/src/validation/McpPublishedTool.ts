// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpPublishedTool} from "../models/McpPublishedTool.js";

export const McpPublishedToolSchema: z.ZodType<McpPublishedTool> = z.strictObject({
  "description": z.string().exactOptional(),
  "name": z.string(),
});

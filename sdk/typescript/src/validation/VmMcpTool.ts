// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmMcpTool} from "../models/VmMcpTool.js";

export const VmMcpToolSchema: z.ZodType<VmMcpTool> = z.object({
  "annotations": z.json().exactOptional(),
  "definition_digest": z.string(),
  "description": z.string().nullable().exactOptional(),
  "input_schema": z.json(),
  "namespaced_name": z.string(),
  "original_name": z.string(),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmMcpToolsResponse} from "../models/VmMcpToolsResponse.js";
import {VmMcpToolSchema} from "./VmMcpTool.js";

export const VmMcpToolsResponseSchema: z.ZodType<VmMcpToolsResponse> = z.object({
  "owner_generation": z.uuid(),
  "server_id": z.string(),
  "tools": z.array(z.lazy(() => VmMcpToolSchema)),
  "vm_id": z.string(),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmMcpServer} from "../models/VmMcpServer.js";
import {VmMcpServerKindSchema} from "./VmMcpServerKind.js";

export const VmMcpServerSchema: z.ZodType<VmMcpServer> = z.object({
  "connected": z.boolean(),
  "enabled": z.boolean(),
  "kind": z.lazy(() => VmMcpServerKindSchema),
  "name": z.string(),
  "tool_count": z.int().min(0),
});

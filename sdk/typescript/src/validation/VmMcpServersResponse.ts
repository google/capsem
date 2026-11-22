// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmMcpServersResponse} from "../models/VmMcpServersResponse.js";
import {VmMcpServerSchema} from "./VmMcpServer.js";

export const VmMcpServersResponseSchema: z.ZodType<VmMcpServersResponse> = z.object({
  "owner_generation": z.uuid(),
  "servers": z.array(z.lazy(() => VmMcpServerSchema)),
  "vm_id": z.string(),
});

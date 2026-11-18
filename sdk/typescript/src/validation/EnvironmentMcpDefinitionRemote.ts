// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentMcpDefinitionRemote} from "../models/EnvironmentMcpDefinitionRemote.js";

export const EnvironmentMcpDefinitionRemoteSchema: z.ZodType<EnvironmentMcpDefinitionRemote> = z.object({
  "binding_id": z.string().nullable().exactOptional(),
  "headers": z.record(z.string(), z.string()).exactOptional(),
  "kind": z.literal("remote"),
  "url": z.string(),
});

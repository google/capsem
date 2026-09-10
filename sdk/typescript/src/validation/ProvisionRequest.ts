// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProvisionRequest} from "../models/ProvisionRequest.js";

export const ProvisionRequestSchema: z.ZodType<ProvisionRequest> = z.object({
  "cpus": z.int().min(0).nullable().exactOptional(),
  "env": z.record(z.string(), z.string()).nullable().exactOptional(),
  "from": z.string().nullable().exactOptional(),
  "name": z.string().nullable().exactOptional(),
  "persistent": z.boolean().exactOptional(),
  "profile_id": z.string(),
  "ram_mb": z.int().min(0).nullable().exactOptional(),
});

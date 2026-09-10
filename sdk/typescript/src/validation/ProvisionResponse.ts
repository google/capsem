// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProvisionResponse} from "../models/ProvisionResponse.js";
import {VmActionSchema} from "./VmAction.js";
import {VmLifecycleStateSchema} from "./VmLifecycleState.js";

export const ProvisionResponseSchema: z.ZodType<ProvisionResponse> = z.object({
  "available_actions": z.array(z.lazy(() => VmActionSchema)),
  "can_resume": z.boolean().exactOptional(),
  "id": z.string(),
  "name": z.string(),
  "persistent": z.boolean().exactOptional(),
  "profile_id": z.string(),
  "status": z.lazy(() => VmLifecycleStateSchema),
  "uds_path": z.string().nullable().exactOptional(),
});

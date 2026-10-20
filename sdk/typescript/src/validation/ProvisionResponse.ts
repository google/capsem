// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProvisionResponse} from "../models/ProvisionResponse.js";
import {ManagedSessionOwnershipSchema} from "./ManagedSessionOwnership.js";
import {VmActionSchema} from "./VmAction.js";
import {VmLifecycleStateSchema} from "./VmLifecycleState.js";

export const ProvisionResponseSchema: z.ZodType<ProvisionResponse> = z.object({
  "available_actions": z.array(z.lazy(() => VmActionSchema)),
  "can_resume": z.boolean().exactOptional(),
  "id": z.string(),
  "name": z.string(),
  "ownership": z.union([z.null(), z.lazy(() => ManagedSessionOwnershipSchema)]).exactOptional(),
  "persistent": z.boolean().exactOptional(),
  "status": z.lazy(() => VmLifecycleStateSchema),
});

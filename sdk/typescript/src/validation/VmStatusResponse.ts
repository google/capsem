// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmStatusResponse} from "../models/VmStatusResponse.js";
import {StorageDiagnosticsSchema} from "./StorageDiagnostics.js";
import {VmActionSchema} from "./VmAction.js";
import {VmLifecycleStateSchema} from "./VmLifecycleState.js";

export const VmStatusResponseSchema: z.ZodType<VmStatusResponse> = z.object({
  "available_actions": z.array(z.lazy(() => VmActionSchema)),
  "can_resume": z.boolean().exactOptional(),
  "created_at": z.string().nullable().exactOptional(),
  "id": z.string(),
  "last_error": z.string().nullable().exactOptional(),
  "name": z.string(),
  "persistent": z.boolean().exactOptional(),
  "pid": z.int().min(0).nullable().exactOptional(),
  "resume_blocked_reason": z.string().nullable().exactOptional(),
  "status": z.lazy(() => VmLifecycleStateSchema),
  "storage": z.union([z.null(), z.lazy(() => StorageDiagnosticsSchema)]).exactOptional(),
  "uptime_secs": z.int().min(0).nullable().exactOptional(),
});

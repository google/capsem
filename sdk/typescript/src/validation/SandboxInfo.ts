// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SandboxInfo} from "../models/SandboxInfo.js";
import {SessionDbStatusSchema} from "./SessionDbStatus.js";
import {StorageDiagnosticsSchema} from "./StorageDiagnostics.js";
import {VmActionSchema} from "./VmAction.js";
import {VmAiInfoSchema} from "./VmAiInfo.js";
import {VmFilesInfoSchema} from "./VmFilesInfo.js";
import {VmLifecycleStateSchema} from "./VmLifecycleState.js";
import {VmNetworkInfoSchema} from "./VmNetworkInfo.js";

export const SandboxInfoSchema: z.ZodType<SandboxInfo> = z.object({
  "ai": z.union([z.null(), z.lazy(() => VmAiInfoSchema)]).exactOptional(),
  "allowed_requests": z.int().min(0).nullable().exactOptional(),
  "available_actions": z.array(z.lazy(() => VmActionSchema)),
  "can_resume": z.boolean().exactOptional(),
  "cpus": z.int().min(0).nullable().exactOptional(),
  "created_at": z.string().nullable().exactOptional(),
  "denied_requests": z.int().min(0).nullable().exactOptional(),
  "description": z.string().nullable().exactOptional(),
  "files": z.union([z.null(), z.lazy(() => VmFilesInfoSchema)]).exactOptional(),
  "forked_from": z.string().nullable().exactOptional(),
  "id": z.string(),
  "last_error": z.string().nullable().exactOptional(),
  "model_call_count": z.int().min(0).nullable().exactOptional(),
  "name": z.string().nullable().exactOptional(),
  "network": z.union([z.null(), z.lazy(() => VmNetworkInfoSchema)]).exactOptional(),
  "persistent": z.boolean().exactOptional(),
  "pid": z.int().min(0),
  "profile_id": z.string(),
  "ram_mb": z.int().min(0).nullable().exactOptional(),
  "resume_blocked_reason": z.string().nullable().exactOptional(),
  "session_db": z.union([z.null(), z.lazy(() => SessionDbStatusSchema)]).exactOptional(),
  "size_bytes": z.int().min(0).nullable().exactOptional(),
  "status": z.lazy(() => VmLifecycleStateSchema),
  "storage": z.union([z.null(), z.lazy(() => StorageDiagnosticsSchema)]).exactOptional(),
  "total_estimated_cost": z.number().nullable().exactOptional(),
  "total_file_events": z.int().min(0).nullable().exactOptional(),
  "total_input_tokens": z.int().min(0).nullable().exactOptional(),
  "total_output_tokens": z.int().min(0).nullable().exactOptional(),
  "total_requests": z.int().min(0).nullable().exactOptional(),
  "total_thinking_tokens": z.int().min(0).nullable().exactOptional(),
  "total_tool_calls": z.int().min(0).nullable().exactOptional(),
  "uptime_secs": z.int().min(0).nullable().exactOptional(),
  "version": z.string().nullable().exactOptional(),
});

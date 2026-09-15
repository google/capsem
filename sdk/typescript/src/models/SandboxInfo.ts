// Generated from Capsem OpenAPI. Do not edit.

import type { SessionDbStatus } from "./SessionDbStatus.js";
import type { StorageDiagnostics } from "./StorageDiagnostics.js";
import type { VmAction } from "./VmAction.js";
import type { VmAiInfo } from "./VmAiInfo.js";
import type { VmFilesInfo } from "./VmFilesInfo.js";
import type { VmLifecycleState } from "./VmLifecycleState.js";
import type { VmNetworkInfo } from "./VmNetworkInfo.js";

export interface SandboxInfo {
  "ai"?: null | VmAiInfo;
  "allowed_requests"?: number | null;
  "available_actions": Array<VmAction>;
  "can_resume"?: boolean;
  "cpus"?: number | null;
  "created_at"?: string | null;
  "denied_requests"?: number | null;
  "description"?: string | null;
  "files"?: null | VmFilesInfo;
  "forked_from"?: string | null;
  "id": string;
  "last_error"?: string | null;
  "model_call_count"?: number | null;
  "name"?: string | null;
  "network"?: null | VmNetworkInfo;
  "persistent"?: boolean;
  "pid": number;
  "profile_id": string;
  "ram_mb"?: number | null;
  "resume_blocked_reason"?: string | null;
  "session_db"?: null | SessionDbStatus;
  "size_bytes"?: number | null;
  "status": VmLifecycleState;
  "storage"?: null | StorageDiagnostics;
  "total_estimated_cost"?: number | null;
  "total_file_events"?: number | null;
  "total_input_tokens"?: number | null;
  "total_output_tokens"?: number | null;
  "total_requests"?: number | null;
  "total_thinking_tokens"?: number | null;
  "total_tool_calls"?: number | null;
  "uptime_secs"?: number | null;
  "version"?: string | null;
}

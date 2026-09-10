// Generated from Capsem OpenAPI. Do not edit.

import type { StorageDiagnostics } from "./StorageDiagnostics.js";
import type { VmAction } from "./VmAction.js";
import type { VmLifecycleState } from "./VmLifecycleState.js";

export interface VmStatusResponse {
  "available_actions": Array<VmAction>;
  "can_resume"?: boolean;
  "created_at"?: string | null;
  "id": string;
  "last_error"?: string | null;
  "name": string;
  "persistent"?: boolean;
  "pid"?: number | null;
  "resume_blocked_reason"?: string | null;
  "status": VmLifecycleState;
  "storage"?: null | StorageDiagnostics;
  "uptime_secs"?: number | null;
}

// Generated from Capsem OpenAPI. Do not edit.

import type { ExecSource } from "./ExecSource.js";

export interface ProcessEvent {
  "command": string;
  "credential_ref"?: string | null;
  "duration_ms"?: number | null;
  "event_id": string;
  "exec_id": number;
  "exit_code"?: number | null;
  "pid"?: number | null;
  "process_name"?: string | null;
  "source": ExecSource;
  "stderr_bytes"?: number | null;
  "stdout_bytes"?: number | null;
  "timestamp": string;
  "trace_id"?: string | null;
}

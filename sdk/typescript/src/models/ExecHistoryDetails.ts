// Generated from Capsem OpenAPI. Do not edit.

import type { ExecSource } from "./ExecSource.js";

export interface ExecHistoryDetails {
  "exec_id": number;
  "process_name"?: string | null;
  "source": ExecSource;
  "trace_id"?: string | null;
}

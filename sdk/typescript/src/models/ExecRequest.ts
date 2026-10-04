// Generated from Capsem OpenAPI. Do not edit.

import type { ExecTarget } from "./ExecTarget.js";

export interface ExecRequest {
  "command": string;
  "target"?: null | ExecTarget;
  "timeout_secs"?: number | null;
}

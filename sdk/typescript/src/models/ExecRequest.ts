// Generated from Capsem OpenAPI. Do not edit.

import type { ExecTarget } from "./ExecTarget.js";

export interface ExecRequest {
  "command": string;
  "cwd"?: string | null;
  "target"?: null | ExecTarget;
  "timeout_secs"?: number | null;
  "user"?: string | null;
}

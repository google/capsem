// Generated from Capsem OpenAPI. Do not edit.

import type { ExecOutput } from "./ExecOutput.js";

export interface ExecResponse {
  "exit_code": number;
  "stderr": ExecOutput;
  "stdout": ExecOutput;
  "truncated"?: boolean;
}

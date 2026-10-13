// Generated from Capsem OpenAPI. Do not edit.

import type { ErrorCode } from "./ErrorCode.js";

export interface ErrorResponse {
  "code"?: null | ErrorCode;
  "error": string;
  "timeout_secs"?: number | null;
  "vm_id"?: string | null;
}

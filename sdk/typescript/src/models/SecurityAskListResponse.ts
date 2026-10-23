// Generated from Capsem OpenAPI. Do not edit.

import type { SecurityAskRequest } from "./SecurityAskRequest.js";

export interface SecurityAskListResponse {
  "generation": string;
  "pending_count": number;
  "requests": Array<SecurityAskRequest>;
  "vm_id": string;
}

// Generated from Capsem OpenAPI. Do not edit.

import type { ConnectionSummary } from "./ConnectionSummary.js";
import type { LocalCredentialAuthority } from "./LocalCredentialAuthority.js";

export interface DisconnectResponse {
  "connection": ConnectionSummary;
  "local_authority": LocalCredentialAuthority;
  "revoked_grant_count": number;
}

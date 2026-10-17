// Generated from Capsem OpenAPI. Do not edit.

import type { RevokedCredentialAuthority } from "./RevokedCredentialAuthority.js";
import type { SessionGrantSummary } from "./SessionGrantSummary.js";

export interface RevokeSessionGrantResponse {
  "grant": SessionGrantSummary;
  "local_authority": RevokedCredentialAuthority;
}

// Generated from Capsem OpenAPI. Do not edit.

import type { ConnectionProvider } from "./ConnectionProvider.js";
import type { ConnectionState } from "./ConnectionState.js";
import type { CredentialErrorCode } from "./CredentialErrorCode.js";
import type { ProviderRevocationState } from "./ProviderRevocationState.js";

export interface ConnectionSummary {
  "account_label": string | null;
  "available_capabilities": Array<string>;
  "created_at": string;
  "error_code": null | CredentialErrorCode;
  "granted_scopes": Array<string>;
  "id": string;
  "last_refresh_at": string | null;
  "provider": ConnectionProvider;
  "provider_revocation": ProviderRevocationState;
  "revision": number;
  "scope_profile_id": string;
  "state": ConnectionState;
  "updated_at": string;
}

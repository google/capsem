// Generated from Capsem OpenAPI. Do not edit.

import type { CredentialErrorCode } from "./CredentialErrorCode.js";
import type { TransactionState } from "./TransactionState.js";

export interface AuthorizationTransaction {
  "connection_id": string | null;
  "error_code": null | CredentialErrorCode;
  "expires_at": string;
  "id": string;
  "state": TransactionState;
}

// Generated from Capsem OpenAPI. Do not edit.

import type { CredentialInjectProvider } from "./CredentialInjectProvider.js";
import type { CredentialStorage } from "./CredentialStorage.js";

export interface CredentialInjectRequest {
  "provider": CredentialInjectProvider;
  "storage"?: CredentialStorage;
  "value": string;
}

// Generated from Capsem OpenAPI. Do not edit.

import type { CredentialEventType } from "./CredentialEventType.js";
import type { CredentialOutcome } from "./CredentialOutcome.js";
import type { MaterialClass } from "./MaterialClass.js";

export interface CredentialEvent {
  "context_json"?: string | null;
  "event_id": string;
  "event_type"?: null | CredentialEventType;
  "material_class": MaterialClass;
  "origin"?: null | CredentialEventType;
  "provider"?: string | null;
  "source": string;
  "timestamp": string;
  "trace_id"?: string | null;
  "verb": CredentialOutcome;
}

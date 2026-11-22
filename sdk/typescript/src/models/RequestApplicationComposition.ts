// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface RequestApplicationComposition {
  "boot": EnvironmentRevisionReference;
  "corporate_policy_id": string;
  "desired": EnvironmentRevisionReference;
  "live_policy": EnvironmentRevisionReference;
  "pending_restart": boolean;
}

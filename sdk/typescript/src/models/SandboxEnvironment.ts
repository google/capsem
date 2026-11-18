// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface SandboxEnvironment {
  "boot"?: null | EnvironmentRevisionReference;
  "desired": EnvironmentRevisionReference;
  "live_policy"?: null | EnvironmentRevisionReference;
  "owner_generation"?: string | null;
  "pending_restart"?: boolean | null;
  "policy_digest"?: string | null;
}

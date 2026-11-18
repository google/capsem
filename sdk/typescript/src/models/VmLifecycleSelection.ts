// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface VmLifecycleSelection {
  "environment"?: null | EnvironmentRevisionReference;
  "generation"?: string | null;
  "id": string;
  "running": boolean;
}

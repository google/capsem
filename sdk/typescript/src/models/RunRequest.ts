// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";

export interface RunRequest {
  "command": string;
  "cpus"?: number | null;
  "env"?: Record<string, string> | null;
  "environment"?: null | EnvironmentRevisionReference;
  "ram_mb"?: number | null;
  "timeout_secs"?: number | null;
}

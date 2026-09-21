// Generated from Capsem OpenAPI. Do not edit.

import type { ContainerSpec } from "./ContainerSpec.js";

export interface ProvisionRequest {
  "container"?: null | ContainerSpec;
  "cpus"?: number | null;
  "env"?: Record<string, string> | null;
  "from"?: string | null;
  "name"?: string | null;
  "networks"?: Array<string>;
  "persistent"?: boolean;
  "profile_id": string;
  "ram_mb"?: number | null;
}

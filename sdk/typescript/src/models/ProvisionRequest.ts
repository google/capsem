// Generated from Capsem OpenAPI. Do not edit.

import type { ContainerSpec } from "./ContainerSpec.js";
import type { EnvironmentRevisionReference } from "./EnvironmentRevisionReference.js";
import type { ManagedSessionCreate } from "./ManagedSessionCreate.js";
import type { VmLifecycleSelection } from "./VmLifecycleSelection.js";

export interface ProvisionRequest {
  "attach"?: boolean;
  "container"?: null | ContainerSpec;
  "cpus"?: number | null;
  "env"?: Record<string, string> | null;
  "environment"?: null | EnvironmentRevisionReference;
  "expected_source"?: null | VmLifecycleSelection;
  "from"?: string | null;
  "labels"?: Record<string, string> | null;
  "managed"?: null | ManagedSessionCreate;
  "name"?: string | null;
  "networks"?: Array<string>;
  "persistent"?: boolean;
  "ram_mb"?: number | null;
}

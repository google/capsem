// Generated from Capsem OpenAPI. Do not edit.

import type { RegistryAccess } from "./RegistryAccess.js";

export interface ContainerSpec {
  "args"?: Array<string>;
  "env"?: Record<string, string>;
  "image": string;
  "registry"?: null | RegistryAccess;
}

// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentSecretBinding } from "./EnvironmentSecretBinding.js";

export interface EnvironmentMcpDefinitionSandbox {
  "args"?: Array<string>;
  "executable": string;
  "kind": "sandbox";
  "secrets"?: Record<string, EnvironmentSecretBinding>;
  "variables"?: Record<string, string>;
  "workdir"?: string | null;
}

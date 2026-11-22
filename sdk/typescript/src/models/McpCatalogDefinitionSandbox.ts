// Generated from Capsem OpenAPI. Do not edit.

import type { McpCatalogRequirement } from "./McpCatalogRequirement.js";

export interface McpCatalogDefinitionSandbox {
  "args"?: Array<string>;
  "executable": string;
  "kind": "sandbox";
  "secret_variables"?: Record<string, McpCatalogRequirement>;
  "variables"?: Record<string, McpCatalogRequirement>;
  "workdir"?: string | null;
}

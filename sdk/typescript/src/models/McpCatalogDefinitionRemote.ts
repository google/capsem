// Generated from Capsem OpenAPI. Do not edit.

import type { McpCatalogRequirement } from "./McpCatalogRequirement.js";

export interface McpCatalogDefinitionRemote {
  "headers"?: Record<string, McpCatalogRequirement>;
  "kind": "remote";
  "secret_binding"?: null | McpCatalogRequirement;
  "url": string;
}

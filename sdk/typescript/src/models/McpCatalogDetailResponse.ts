// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogEntryMetadata } from "./CatalogEntryMetadata.js";
import type { EnvironmentCatalogPin } from "./EnvironmentCatalogPin.js";
import type { McpCatalogDefinition } from "./McpCatalogDefinition.js";
import type { McpPublishedTool } from "./McpPublishedTool.js";

export interface McpCatalogDetailResponse {
  "definition": McpCatalogDefinition;
  "metadata": CatalogEntryMetadata;
  "published_tools"?: Array<McpPublishedTool> | null;
  "selection": EnvironmentCatalogPin;
}

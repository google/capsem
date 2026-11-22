// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogSnapshot } from "./CatalogSnapshot.js";
import type { McpCatalogSummary } from "./McpCatalogSummary.js";

export interface McpCatalogListResponse {
  "catalog": CatalogSnapshot;
  "mcps": Array<McpCatalogSummary>;
  "next_cursor"?: string | null;
}

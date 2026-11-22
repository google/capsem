// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogSnapshot } from "./CatalogSnapshot.js";
import type { RuleCatalogSummary } from "./RuleCatalogSummary.js";

export interface RuleCatalogListResponse {
  "catalog": CatalogSnapshot;
  "next_cursor"?: string | null;
  "rules": Array<RuleCatalogSummary>;
}

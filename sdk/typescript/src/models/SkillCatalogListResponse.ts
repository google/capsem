// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogEntryMetadata } from "./CatalogEntryMetadata.js";
import type { CatalogSnapshot } from "./CatalogSnapshot.js";

export interface SkillCatalogListResponse {
  "catalog": CatalogSnapshot;
  "next_cursor"?: string | null;
  "skills": Array<CatalogEntryMetadata>;
}

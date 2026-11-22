// Generated from Capsem OpenAPI. Do not edit.

import type { CatalogEntryMetadata } from "./CatalogEntryMetadata.js";
import type { SkillCatalogSnapshot } from "./SkillCatalogSnapshot.js";

export interface SkillCatalogListResponse {
  "catalog": SkillCatalogSnapshot;
  "next_cursor"?: string | null;
  "skills": Array<CatalogEntryMetadata>;
}

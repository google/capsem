// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SkillCatalogListResponse} from "../models/SkillCatalogListResponse.js";
import {CatalogEntryMetadataSchema} from "./CatalogEntryMetadata.js";
import {CatalogSnapshotSchema} from "./CatalogSnapshot.js";

export const SkillCatalogListResponseSchema: z.ZodType<SkillCatalogListResponse> = z.strictObject({
  "catalog": z.lazy(() => CatalogSnapshotSchema),
  "next_cursor": z.string().nullable().exactOptional(),
  "skills": z.array(z.lazy(() => CatalogEntryMetadataSchema)),
});

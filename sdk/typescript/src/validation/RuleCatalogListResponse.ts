// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RuleCatalogListResponse} from "../models/RuleCatalogListResponse.js";
import {CatalogSnapshotSchema} from "./CatalogSnapshot.js";
import {RuleCatalogSummarySchema} from "./RuleCatalogSummary.js";

export const RuleCatalogListResponseSchema: z.ZodType<RuleCatalogListResponse> = z.strictObject({
  "catalog": z.lazy(() => CatalogSnapshotSchema),
  "next_cursor": z.string().nullable().exactOptional(),
  "rules": z.array(z.lazy(() => RuleCatalogSummarySchema)),
});

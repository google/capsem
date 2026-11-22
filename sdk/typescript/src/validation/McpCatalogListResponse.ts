// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpCatalogListResponse} from "../models/McpCatalogListResponse.js";
import {CatalogSnapshotSchema} from "./CatalogSnapshot.js";
import {McpCatalogSummarySchema} from "./McpCatalogSummary.js";

export const McpCatalogListResponseSchema: z.ZodType<McpCatalogListResponse> = z.strictObject({
  "catalog": z.lazy(() => CatalogSnapshotSchema),
  "mcps": z.array(z.lazy(() => McpCatalogSummarySchema)),
  "next_cursor": z.string().nullable().exactOptional(),
});

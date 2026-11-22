// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpCatalogSummary} from "../models/McpCatalogSummary.js";
import {CatalogEntryMetadataSchema} from "./CatalogEntryMetadata.js";
import {McpCatalogKindSchema} from "./McpCatalogKind.js";

export const McpCatalogSummarySchema: z.ZodType<McpCatalogSummary> = z.strictObject({
  "kind": z.lazy(() => McpCatalogKindSchema),
  "metadata": z.lazy(() => CatalogEntryMetadataSchema),
});

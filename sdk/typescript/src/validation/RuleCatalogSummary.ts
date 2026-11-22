// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RuleCatalogSummary} from "../models/RuleCatalogSummary.js";
import {CatalogEntryMetadataSchema} from "./CatalogEntryMetadata.js";
import {CatalogRuleKindSchema} from "./CatalogRuleKind.js";

export const RuleCatalogSummarySchema: z.ZodType<RuleCatalogSummary> = z.strictObject({
  "kind": z.lazy(() => CatalogRuleKindSchema),
  "metadata": z.lazy(() => CatalogEntryMetadataSchema),
});

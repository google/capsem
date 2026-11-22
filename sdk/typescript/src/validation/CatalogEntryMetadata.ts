// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogEntryMetadata} from "../models/CatalogEntryMetadata.js";
import {CatalogCompatibilitySchema} from "./CatalogCompatibility.js";
import {CatalogEntryStatusSchema} from "./CatalogEntryStatus.js";

export const CatalogEntryMetadataSchema: z.ZodType<CatalogEntryMetadata> = z.strictObject({
  "compatibility": z.lazy(() => CatalogCompatibilitySchema),
  "description": z.string().exactOptional(),
  "id": z.string(),
  "license": z.string(),
  "name": z.string(),
  "provenance": z.string(),
  "publisher": z.string(),
  "status": z.lazy(() => CatalogEntryStatusSchema),
  "status_reason": z.string().nullable().exactOptional(),
  "tags": z.array(z.string()).exactOptional(),
  "version": z.string(),
});

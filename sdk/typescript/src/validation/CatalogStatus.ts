// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogStatus} from "../models/CatalogStatus.js";
import {CatalogAvailabilitySchema} from "./CatalogAvailability.js";
import {CatalogKindSchema} from "./CatalogKind.js";
import {CatalogRefreshErrorSchema} from "./CatalogRefreshError.js";
import {CatalogSourceOriginSchema} from "./CatalogSourceOrigin.js";

export const CatalogStatusSchema: z.ZodType<CatalogStatus> = z.strictObject({
  "availability": z.lazy(() => CatalogAvailabilitySchema),
  "digest": z.string().nullable(),
  "error": z.union([z.null(), z.lazy(() => CatalogRefreshErrorSchema)]),
  "kind": z.lazy(() => CatalogKindSchema),
  "last_successful_refresh": z.int().nullable(),
  "origin": z.union([z.null(), z.lazy(() => CatalogSourceOriginSchema)]),
  "reference": z.string().nullable(),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSkillDetailResponse} from "../models/EnvironmentSkillDetailResponse.js";
import {CatalogEntryMetadataSchema} from "./CatalogEntryMetadata.js";
import {EnvironmentCatalogPinSchema} from "./EnvironmentCatalogPin.js";
import {EnvironmentRevisionReferenceSchema} from "./EnvironmentRevisionReference.js";

export const EnvironmentSkillDetailResponseSchema: z.ZodType<EnvironmentSkillDetailResponse> = z.strictObject({
  "discovery_fresh": z.boolean(),
  "files": z.record(z.string(), z.string()),
  "metadata": z.lazy(() => CatalogEntryMetadataSchema),
  "revision": z.lazy(() => EnvironmentRevisionReferenceSchema),
  "selection": z.lazy(() => EnvironmentCatalogPinSchema),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SkillCatalogDetailResponse} from "../models/SkillCatalogDetailResponse.js";
import {CatalogEntryMetadataSchema} from "./CatalogEntryMetadata.js";
import {EnvironmentCatalogPinSchema} from "./EnvironmentCatalogPin.js";

export const SkillCatalogDetailResponseSchema: z.ZodType<SkillCatalogDetailResponse> = z.strictObject({
  "files": z.record(z.string(), z.string()),
  "metadata": z.lazy(() => CatalogEntryMetadataSchema),
  "selection": z.lazy(() => EnvironmentCatalogPinSchema),
});

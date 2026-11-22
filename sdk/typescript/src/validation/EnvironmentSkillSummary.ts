// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentSkillSummary} from "../models/EnvironmentSkillSummary.js";
import {CatalogEntryMetadataSchema} from "./CatalogEntryMetadata.js";
import {EnvironmentCatalogPinSchema} from "./EnvironmentCatalogPin.js";
import {EnvironmentSkillAvailabilitySchema} from "./EnvironmentSkillAvailability.js";

export const EnvironmentSkillSummarySchema: z.ZodType<EnvironmentSkillSummary> = z.strictObject({
  "availability": z.lazy(() => EnvironmentSkillAvailabilitySchema),
  "metadata": z.union([z.null(), z.lazy(() => CatalogEntryMetadataSchema)]).exactOptional(),
  "selection": z.lazy(() => EnvironmentCatalogPinSchema),
});

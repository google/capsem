// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RuleCatalogDetailResponse} from "../models/RuleCatalogDetailResponse.js";
import {CatalogEntryMetadataSchema} from "./CatalogEntryMetadata.js";
import {CatalogRuleParameterSchema} from "./CatalogRuleParameter.js";
import {EnvironmentCatalogPinSchema} from "./EnvironmentCatalogPin.js";
import {RuleCatalogDefinitionSchema} from "./RuleCatalogDefinition.js";

export const RuleCatalogDetailResponseSchema: z.ZodType<RuleCatalogDetailResponse> = z.strictObject({
  "definition": z.lazy(() => RuleCatalogDefinitionSchema),
  "metadata": z.lazy(() => CatalogEntryMetadataSchema),
  "parameters": z.record(z.string(), z.lazy(() => CatalogRuleParameterSchema)),
  "selection": z.lazy(() => EnvironmentCatalogPinSchema),
});

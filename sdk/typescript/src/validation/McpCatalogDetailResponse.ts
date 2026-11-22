// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpCatalogDetailResponse} from "../models/McpCatalogDetailResponse.js";
import {CatalogEntryMetadataSchema} from "./CatalogEntryMetadata.js";
import {EnvironmentCatalogPinSchema} from "./EnvironmentCatalogPin.js";
import {McpCatalogDefinitionSchema} from "./McpCatalogDefinition.js";
import {McpPublishedToolSchema} from "./McpPublishedTool.js";

export const McpCatalogDetailResponseSchema: z.ZodType<McpCatalogDetailResponse> = z.strictObject({
  "definition": z.lazy(() => McpCatalogDefinitionSchema),
  "metadata": z.lazy(() => CatalogEntryMetadataSchema),
  "published_tools": z.array(z.lazy(() => McpPublishedToolSchema)).nullable().exactOptional(),
  "selection": z.lazy(() => EnvironmentCatalogPinSchema),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItem} from "../models/EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItem.js";

export const EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItemSchema: z.ZodType<EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItem> = z.strictObject({
  "catalog_reference": z.string(),
  "id": z.string(),
  "resolved_digest": z.string(),
  "version": z.string(),
});

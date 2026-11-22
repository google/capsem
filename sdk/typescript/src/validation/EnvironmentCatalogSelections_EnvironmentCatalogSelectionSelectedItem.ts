// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentCatalogSelections_EnvironmentCatalogSelectionSelectedItem} from "../models/EnvironmentCatalogSelections_EnvironmentCatalogSelectionSelectedItem.js";
import {EnvironmentCatalogExpectationSchema} from "./EnvironmentCatalogExpectation.js";

export const EnvironmentCatalogSelections_EnvironmentCatalogSelectionSelectedItemSchema: z.ZodType<EnvironmentCatalogSelections_EnvironmentCatalogSelectionSelectedItem> = z.strictObject({
  "expected_catalog": z.union([z.null(), z.lazy(() => EnvironmentCatalogExpectationSchema)]).exactOptional(),
  "id": z.string(),
  "version": z.string(),
});

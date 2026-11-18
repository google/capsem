// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentCatalogSelections_EnvironmentCatalogSelection} from "../models/EnvironmentCatalogSelections_EnvironmentCatalogSelection.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogSelectionSelectedItemSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogSelectionSelectedItem.js";

export const EnvironmentCatalogSelections_EnvironmentCatalogSelectionSchema: z.ZodType<EnvironmentCatalogSelections_EnvironmentCatalogSelection> = z.strictObject({
  "selected": z.array(z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogSelectionSelectedItemSchema)).exactOptional(),
});

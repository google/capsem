// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentCatalogSelections_EnvironmentCatalogPin} from "../models/EnvironmentCatalogSelections_EnvironmentCatalogPin.js";
import {EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItemSchema} from "./EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItem.js";

export const EnvironmentCatalogSelections_EnvironmentCatalogPinSchema: z.ZodType<EnvironmentCatalogSelections_EnvironmentCatalogPin> = z.strictObject({
  "selected": z.array(z.lazy(() => EnvironmentCatalogSelections_EnvironmentCatalogPinSelectedItemSchema)).exactOptional(),
});

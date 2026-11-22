// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentCatalogPin} from "../models/EnvironmentCatalogPin.js";

export const EnvironmentCatalogPinSchema: z.ZodType<EnvironmentCatalogPin> = z.strictObject({
  "catalog_reference": z.string(),
  "id": z.string(),
  "resolved_digest": z.string(),
  "version": z.string(),
});

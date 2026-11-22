// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentCatalogExpectation} from "../models/EnvironmentCatalogExpectation.js";

export const EnvironmentCatalogExpectationSchema: z.ZodType<EnvironmentCatalogExpectation> = z.strictObject({
  "digest": z.string(),
  "reference": z.string(),
});

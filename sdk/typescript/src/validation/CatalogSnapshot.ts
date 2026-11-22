// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CatalogSnapshot} from "../models/CatalogSnapshot.js";

export const CatalogSnapshotSchema: z.ZodType<CatalogSnapshot> = z.strictObject({
  "digest": z.string(),
  "reference": z.string(),
  "snapshot": z.string(),
});

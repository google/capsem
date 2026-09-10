// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SupplyChainReference} from "../models/SupplyChainReference.js";

export const SupplyChainReferenceSchema: z.ZodType<SupplyChainReference> = z.object({
  "format": z.string().nullable().exactOptional(),
  "generator": z.string().nullable().exactOptional(),
  "name": z.string(),
  "release_artifact": z.string().nullable().exactOptional(),
  "route": z.string().nullable().exactOptional(),
  "scope": z.string().nullable().exactOptional(),
  "workflow": z.string().nullable().exactOptional(),
});

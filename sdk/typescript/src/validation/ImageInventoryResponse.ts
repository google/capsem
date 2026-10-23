// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImageInventoryResponse} from "../models/ImageInventoryResponse.js";
import {ImagePackageInventorySchema} from "./ImagePackageInventory.js";

export const ImageInventoryResponseSchema: z.ZodType<ImageInventoryResponse> = z.object({
  "architecture": z.string(),
  "artifact_reference": z.string().nullable().exactOptional(),
  "image": z.string(),
  "inventory": z.union([z.null(), z.lazy(() => ImagePackageInventorySchema)]).exactOptional(),
});

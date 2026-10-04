// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImageListResponse} from "../models/ImageListResponse.js";
import {CatalogInfoSchema} from "./CatalogInfo.js";
import {ImageInfoSchema} from "./ImageInfo.js";

export const ImageListResponseSchema: z.ZodType<ImageListResponse> = z.object({
  "catalog": z.union([z.null(), z.lazy(() => CatalogInfoSchema)]).exactOptional(),
  "images": z.array(z.lazy(() => ImageInfoSchema)),
});

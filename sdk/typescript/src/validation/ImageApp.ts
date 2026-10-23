// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImageApp} from "../models/ImageApp.js";
import {ImageEvidenceSchema} from "./ImageEvidence.js";
import {ImageInterfaceSchema} from "./ImageInterface.js";

export const ImageAppSchema: z.ZodType<ImageApp> = z.object({
  "architecture": z.string(),
  "catalog_name": z.string(),
  "category": z.string().nullable().exactOptional(),
  "description": z.string(),
  "display_name": z.string(),
  "evidence": z.array(z.lazy(() => ImageEvidenceSchema)).exactOptional(),
  "icon": z.string().nullable().exactOptional(),
  "image": z.string(),
  "interfaces": z.array(z.lazy(() => ImageInterfaceSchema)).exactOptional(),
  "publisher": z.string().nullable().exactOptional(),
  "url": z.string().nullable().exactOptional(),
});

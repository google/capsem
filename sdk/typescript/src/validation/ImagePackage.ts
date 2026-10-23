// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImagePackage} from "../models/ImagePackage.js";

export const ImagePackageSchema: z.ZodType<ImagePackage> = z.object({
  "architecture": z.string(),
  "name": z.string(),
  "version": z.string(),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImagePackageInventory} from "../models/ImagePackageInventory.js";
import {ImagePackageSchema} from "./ImagePackage.js";
import {ImageToolSchema} from "./ImageTool.js";

export const ImagePackageInventorySchema: z.ZodType<ImagePackageInventory> = z.object({
  "architecture": z.string(),
  "packages": z.array(z.lazy(() => ImagePackageSchema)),
  "schema": z.string(),
  "subject": z.string(),
  "tools": z.array(z.lazy(() => ImageToolSchema)),
});

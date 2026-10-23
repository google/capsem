// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImageTool} from "../models/ImageTool.js";

export const ImageToolSchema: z.ZodType<ImageTool> = z.object({
  "name": z.string(),
  "version": z.string(),
});

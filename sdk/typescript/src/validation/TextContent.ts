// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TextContent} from "../models/TextContent.js";
import {TextContentKindSchema} from "./TextContentKind.js";

export const TextContentSchema: z.ZodType<TextContent> = z.strictObject({
  "kind": z.lazy(() => TextContentKindSchema),
  "text": z.string(),
});

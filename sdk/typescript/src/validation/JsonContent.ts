// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {JsonContent} from "../models/JsonContent.js";
import {JsonContentKindSchema} from "./JsonContentKind.js";

export const JsonContentSchema: z.ZodType<JsonContent> = z.strictObject({
  "kind": z.lazy(() => JsonContentKindSchema),
  "value": z.json(),
});

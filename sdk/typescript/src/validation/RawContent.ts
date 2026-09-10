// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RawContent} from "../models/RawContent.js";
import {RawContentKindSchema} from "./RawContentKind.js";
import {RawContentReasonSchema} from "./RawContentReason.js";

export const RawContentSchema: z.ZodType<RawContent> = z.strictObject({
  "kind": z.lazy(() => RawContentKindSchema),
  "raw": z.string(),
  "reason": z.lazy(() => RawContentReasonSchema),
});

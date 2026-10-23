// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImageEvidence} from "../models/ImageEvidence.js";
import {ImageEvidenceKindSchema} from "./ImageEvidenceKind.js";

export const ImageEvidenceSchema: z.ZodType<ImageEvidence> = z.object({
  "kind": z.lazy(() => ImageEvidenceKindSchema),
  "media_type": z.string(),
  "reference": z.string(),
});

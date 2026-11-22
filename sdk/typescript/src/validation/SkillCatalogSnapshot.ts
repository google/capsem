// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SkillCatalogSnapshot} from "../models/SkillCatalogSnapshot.js";

export const SkillCatalogSnapshotSchema: z.ZodType<SkillCatalogSnapshot> = z.strictObject({
  "digest": z.string(),
  "reference": z.string(),
  "snapshot": z.string(),
});

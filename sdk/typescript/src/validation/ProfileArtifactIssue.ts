// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfileArtifactIssue} from "../models/ProfileArtifactIssue.js";

export const ProfileArtifactIssueSchema: z.ZodType<ProfileArtifactIssue> = z.object({
  "kind": z.string(),
  "path": z.string(),
  "present": z.boolean().nullable().exactOptional(),
  "valid": z.boolean().nullable().exactOptional(),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfileReadiness} from "../models/ProfileReadiness.js";
import {ProfileArtifactIssueSchema} from "./ProfileArtifactIssue.js";
import {ProfileUpdateSemanticsSchema} from "./ProfileUpdateSemantics.js";

export const ProfileReadinessSchema: z.ZodType<ProfileReadiness> = z.object({
  "asset_count": z.int().min(0),
  "current_arch": z.string(),
  "description": z.string(),
  "errors": z.array(z.string()),
  "id": z.string(),
  "invalid_assets": z.array(z.lazy(() => ProfileArtifactIssueSchema)),
  "invalid_files": z.array(z.lazy(() => ProfileArtifactIssueSchema)),
  "missing_assets": z.array(z.lazy(() => ProfileArtifactIssueSchema)),
  "name": z.string(),
  "profile_payload_hash": z.string().nullable().exactOptional(),
  "ready": z.boolean(),
  "revision": z.string().nullable().exactOptional(),
  "update_semantics": z.union([z.null(), z.lazy(() => ProfileUpdateSemanticsSchema)]).exactOptional(),
});

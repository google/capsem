// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfileSummary} from "../models/ProfileSummary.js";
import {ProfileAvailabilitySummarySchema} from "./ProfileAvailabilitySummary.js";
import {ProfileUpdateSemanticsSchema} from "./ProfileUpdateSemantics.js";

export const ProfileSummarySchema: z.ZodType<ProfileSummary> = z.object({
  "availability": z.lazy(() => ProfileAvailabilitySummarySchema),
  "default_rule_count": z.int().min(0),
  "description": z.string(),
  "icon_svg": z.string().nullable().exactOptional(),
  "id": z.string(),
  "mcp_server_count": z.int().min(0),
  "name": z.string(),
  "plugin_count": z.int().min(0),
  "rule_count": z.int().min(0),
  "source": z.string(),
  "update_semantics": z.lazy(() => ProfileUpdateSemanticsSchema),
});

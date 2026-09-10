// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfileAvailabilitySummary} from "../models/ProfileAvailabilitySummary.js";

export const ProfileAvailabilitySummarySchema: z.ZodType<ProfileAvailabilitySummary> = z.object({
  "mobile": z.boolean(),
  "shell": z.boolean(),
  "web": z.boolean(),
});

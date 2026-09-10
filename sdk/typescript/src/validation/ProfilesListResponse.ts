// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfilesListResponse} from "../models/ProfilesListResponse.js";
import {ProfileSummarySchema} from "./ProfileSummary.js";

export const ProfilesListResponseSchema: z.ZodType<ProfilesListResponse> = z.object({
  "profiles": z.array(z.lazy(() => ProfileSummarySchema)),
});

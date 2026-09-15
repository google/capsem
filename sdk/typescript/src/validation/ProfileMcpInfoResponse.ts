// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfileMcpInfoResponse} from "../models/ProfileMcpInfoResponse.js";

export const ProfileMcpInfoResponseSchema: z.ZodType<ProfileMcpInfoResponse> = z.object({
  "builtin_local_enabled": z.boolean(),
  "manual_server_count": z.int().min(0),
  "profile_id": z.string(),
  "server_count": z.int().min(0),
});

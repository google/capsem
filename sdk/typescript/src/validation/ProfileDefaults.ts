// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfileDefaults} from "../models/ProfileDefaults.js";

export const ProfileDefaultsSchema: z.ZodType<ProfileDefaults> = z.object({
  "container": z.string().nullable().exactOptional(),
  "vm": z.string().nullable().exactOptional(),
});

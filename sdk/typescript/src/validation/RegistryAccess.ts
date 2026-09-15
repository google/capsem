// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RegistryAccess} from "../models/RegistryAccess.js";

export const RegistryAccessSchema: z.ZodType<RegistryAccess> = z.object({
  "ca_pem": z.string().nullable().exactOptional(),
  "password": z.string().nullable().exactOptional(),
  "username": z.string().nullable().exactOptional(),
});

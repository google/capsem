// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CreateProxyResponse} from "../models/CreateProxyResponse.js";

export const CreateProxyResponseSchema: z.ZodType<CreateProxyResponse> = z.object({
  "base_url": z.string(),
  "bind": z.string(),
  "lease_expires_unix_ms": z.int().min(0),
  "lease_token": z.string(),
  "port": z.int().min(0),
  "provider": z.string(),
  "session_id": z.string(),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CreateProxyRequest} from "../models/CreateProxyRequest.js";

export const CreateProxyRequestSchema: z.ZodType<CreateProxyRequest> = z.object({
  "bind": z.string(),
  "port": z.int().min(0),
  "provider": z.string(),
});

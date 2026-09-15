// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PersistRequest} from "../models/PersistRequest.js";

export const PersistRequestSchema: z.ZodType<PersistRequest> = z.object({
  "name": z.string(),
});

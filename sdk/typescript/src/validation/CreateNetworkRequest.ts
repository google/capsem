// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CreateNetworkRequest} from "../models/CreateNetworkRequest.js";

export const CreateNetworkRequestSchema: z.ZodType<CreateNetworkRequest> = z.object({
  "name": z.string(),
});

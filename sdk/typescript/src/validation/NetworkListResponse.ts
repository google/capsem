// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {NetworkListResponse} from "../models/NetworkListResponse.js";
import {NetworkInfoSchema} from "./NetworkInfo.js";

export const NetworkListResponseSchema: z.ZodType<NetworkListResponse> = z.object({
  "networks": z.array(z.lazy(() => NetworkInfoSchema)),
});

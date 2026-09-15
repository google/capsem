// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RestartResponse} from "../models/RestartResponse.js";
import {RestartAuthenticationSchema} from "./RestartAuthentication.js";
import {RestartStatusSchema} from "./RestartStatus.js";
import {ServiceManagerSchema} from "./ServiceManager.js";

export const RestartResponseSchema: z.ZodType<RestartResponse> = z.object({
  "authentication": z.lazy(() => RestartAuthenticationSchema),
  "manager": z.lazy(() => ServiceManagerSchema),
  "status": z.lazy(() => RestartStatusSchema),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentListResponse} from "../models/EnvironmentListResponse.js";
import {EnvironmentListItemSchema} from "./EnvironmentListItem.js";

export const EnvironmentListResponseSchema: z.ZodType<EnvironmentListResponse> = z.strictObject({
  "environments": z.array(z.lazy(() => EnvironmentListItemSchema)),
  "next_cursor": z.string().nullable().exactOptional(),
});

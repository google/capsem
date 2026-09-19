// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PanicsResponse} from "../models/PanicsResponse.js";
import {PanicEventSchema} from "./PanicEvent.js";

export const PanicsResponseSchema: z.ZodType<PanicsResponse> = z.object({
  "panics": z.array(z.lazy(() => PanicEventSchema)),
});

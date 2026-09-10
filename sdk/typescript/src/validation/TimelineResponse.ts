// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TimelineResponse} from "../models/TimelineResponse.js";
import {TimelineEventSchema} from "./TimelineEvent.js";

export const TimelineResponseSchema: z.ZodType<TimelineResponse> = z.object({
  "events": z.array(z.lazy(() => TimelineEventSchema)),
});

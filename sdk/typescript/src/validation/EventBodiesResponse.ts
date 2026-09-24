// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EventBodiesResponse} from "../models/EventBodiesResponse.js";
import {ArchivedEventBodySchema} from "./ArchivedEventBody.js";

export const EventBodiesResponseSchema: z.ZodType<EventBodiesResponse> = z.object({
  "bodies": z.array(z.lazy(() => ArchivedEventBodySchema)),
  "event_id": z.string(),
});

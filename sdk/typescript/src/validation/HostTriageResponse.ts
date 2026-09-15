// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {HostTriageResponse} from "../models/HostTriageResponse.js";
import {ErrorEventSchema} from "./ErrorEvent.js";
import {PanicEventSchema} from "./PanicEvent.js";
import {SlowOpEventSchema} from "./SlowOpEvent.js";

export const HostTriageResponseSchema: z.ZodType<HostTriageResponse> = z.object({
  "errors": z.array(z.lazy(() => ErrorEventSchema)),
  "panics": z.array(z.lazy(() => PanicEventSchema)),
  "slow_ops": z.array(z.lazy(() => SlowOpEventSchema)),
});

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TimelineStatus} from "../models/TimelineStatus.js";
import {ToolDecisionSchema} from "./ToolDecision.js";

export const TimelineStatusSchema: z.ZodType<TimelineStatus> = z.union([z.int(), z.lazy(() => ToolDecisionSchema)]);

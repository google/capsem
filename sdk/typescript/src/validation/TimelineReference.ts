// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {TimelineReference} from "../models/TimelineReference.js";

export const TimelineReferenceSchema: z.ZodType<TimelineReference> = z.union([z.int(), z.string()]);

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExecOutput} from "../models/ExecOutput.js";
import {ExecOutputEncodingSchema} from "./ExecOutputEncoding.js";

export const ExecOutputSchema: z.ZodType<ExecOutput> = z.object({
  "data": z.string(),
  "encoding": z.lazy(() => ExecOutputEncodingSchema),
});

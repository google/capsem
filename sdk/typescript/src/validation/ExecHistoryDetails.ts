// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ExecHistoryDetails} from "../models/ExecHistoryDetails.js";
import {ExecSourceSchema} from "./ExecSource.js";

export const ExecHistoryDetailsSchema: z.ZodType<ExecHistoryDetails> = z.object({
  "exec_id": z.int().min(0),
  "process_name": z.string().nullable().exactOptional(),
  "source": z.lazy(() => ExecSourceSchema),
  "trace_id": z.string().nullable().exactOptional(),
});

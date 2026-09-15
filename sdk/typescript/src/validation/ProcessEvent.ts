// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProcessEvent} from "../models/ProcessEvent.js";
import {ExecSourceSchema} from "./ExecSource.js";

export const ProcessEventSchema: z.ZodType<ProcessEvent> = z.object({
  "command": z.string(),
  "credential_ref": z.string().nullable().exactOptional(),
  "duration_ms": z.int().min(0).nullable().exactOptional(),
  "event_id": z.string(),
  "exec_id": z.int().min(0),
  "exit_code": z.int().nullable().exactOptional(),
  "pid": z.int().min(0).nullable().exactOptional(),
  "process_name": z.string().nullable().exactOptional(),
  "source": z.lazy(() => ExecSourceSchema),
  "stderr_bytes": z.int().min(0).nullable().exactOptional(),
  "stdout_bytes": z.int().min(0).nullable().exactOptional(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
});

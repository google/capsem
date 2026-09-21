// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AuditEvent} from "../models/AuditEvent.js";

export const AuditEventSchema: z.ZodType<AuditEvent> = z.object({
  "argv": z.string(),
  "audit_id": z.string().nullable().exactOptional(),
  "comm": z.string().nullable().exactOptional(),
  "credential_ref": z.string().nullable().exactOptional(),
  "cwd": z.string().nullable().exactOptional(),
  "event_id": z.string(),
  "exe": z.string(),
  "exec_event_id": z.int().nullable().exactOptional(),
  "exit_code": z.int().nullable().exactOptional(),
  "parent_exe": z.string().nullable().exactOptional(),
  "pid": z.int().min(0),
  "ppid": z.int().min(0),
  "session_id": z.int().min(0).nullable().exactOptional(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
  "tty": z.string().nullable().exactOptional(),
  "uid": z.int().min(0),
});

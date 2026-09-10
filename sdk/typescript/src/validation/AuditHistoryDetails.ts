// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AuditHistoryDetails} from "../models/AuditHistoryDetails.js";

export const AuditHistoryDetailsSchema: z.ZodType<AuditHistoryDetails> = z.object({
  "audit_id": z.string().nullable().exactOptional(),
  "comm": z.string().nullable().exactOptional(),
  "cwd": z.string().nullable().exactOptional(),
  "exe": z.string(),
  "parent_exe": z.string().nullable().exactOptional(),
  "pid": z.int().min(0),
  "ppid": z.int().min(0),
  "session_id": z.int().min(0).nullable().exactOptional(),
  "tty": z.string().nullable().exactOptional(),
  "uid": z.int().min(0),
});

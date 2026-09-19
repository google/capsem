// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {HttpEvent} from "../models/HttpEvent.js";
import {NetworkDecisionSchema} from "./NetworkDecision.js";

export const HttpEventSchema: z.ZodType<HttpEvent> = z.object({
  "bytes_received": z.int().min(0).nullable().exactOptional(),
  "bytes_sent": z.int().min(0).nullable().exactOptional(),
  "credential_ref": z.string().nullable().exactOptional(),
  "decision": z.lazy(() => NetworkDecisionSchema),
  "domain": z.string(),
  "duration_ms": z.int().min(0).nullable().exactOptional(),
  "event_id": z.string(),
  "matched_rule": z.string().nullable().exactOptional(),
  "method": z.string().nullable().exactOptional(),
  "path": z.string().nullable().exactOptional(),
  "policy_rule": z.string().nullable().exactOptional(),
  "port": z.int().min(0).nullable().exactOptional(),
  "query": z.string().nullable().exactOptional(),
  "request_headers": z.string().nullable().exactOptional(),
  "response_headers": z.string().nullable().exactOptional(),
  "status_code": z.int().min(0).nullable().exactOptional(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
});

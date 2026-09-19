// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {DnsEvent} from "../models/DnsEvent.js";
import {NetworkDecisionSchema} from "./NetworkDecision.js";
import {NetworkProtocolSchema} from "./NetworkProtocol.js";

export const DnsEventSchema: z.ZodType<DnsEvent> = z.object({
  "credential_ref": z.string().nullable().exactOptional(),
  "decision": z.lazy(() => NetworkDecisionSchema),
  "event_id": z.string(),
  "matched_rule": z.string().nullable().exactOptional(),
  "policy_rule": z.string().nullable().exactOptional(),
  "process_name": z.string().nullable().exactOptional(),
  "qclass": z.int().min(0),
  "qname": z.string(),
  "qtype": z.int().min(0),
  "rcode": z.int().min(0),
  "source_proto": z.union([z.null(), z.lazy(() => NetworkProtocolSchema)]).exactOptional(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
  "upstream_resolver_ms": z.int().min(0).nullable().exactOptional(),
});

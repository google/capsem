// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CredentialEvent} from "../models/CredentialEvent.js";
import {CredentialEventTypeSchema} from "./CredentialEventType.js";
import {CredentialOutcomeSchema} from "./CredentialOutcome.js";
import {MaterialClassSchema} from "./MaterialClass.js";

export const CredentialEventSchema: z.ZodType<CredentialEvent> = z.object({
  "context_json": z.string().nullable().exactOptional(),
  "event_id": z.string(),
  "event_type": z.union([z.null(), z.lazy(() => CredentialEventTypeSchema)]).exactOptional(),
  "material_class": z.lazy(() => MaterialClassSchema),
  "origin": z.union([z.null(), z.lazy(() => CredentialEventTypeSchema)]).exactOptional(),
  "provider": z.string().nullable().exactOptional(),
  "source": z.string(),
  "timestamp": z.string(),
  "trace_id": z.string().nullable().exactOptional(),
  "verb": z.lazy(() => CredentialOutcomeSchema),
});

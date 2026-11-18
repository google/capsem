// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {VmLifecycleIntent} from "../models/VmLifecycleIntent.js";
import type {VmReadinessResponse} from "../models/VmReadinessResponse.js";
import {VmLifecycleIntentSchema} from "../validation/VmLifecycleIntent.js";
import {VmReadinessResponseSchema} from "../validation/VmReadinessResponse.js";

export async function getVmReadiness(
  transport: Transport,
  parameters: {
    "id": string;
    "intent": VmLifecycleIntent;
  },
  options: CallOptions = {},
): Promise<VmReadinessResponse> {
  const input = z.object({
  "id": z.string(),
  "intent": z.lazy(() => VmLifecycleIntentSchema),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/readiness", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"intent": input["intent"]},
  });
  return z.lazy(() => VmReadinessResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {CreateSessionGrantRequest} from "../models/CreateSessionGrantRequest.js";
import type {SessionGrantSummary} from "../models/SessionGrantSummary.js";
import {CreateSessionGrantRequestSchema} from "../validation/CreateSessionGrantRequest.js";
import {SessionGrantSummarySchema} from "../validation/SessionGrantSummary.js";

export async function createSessionCredentialGrant(
  transport: Transport,
  parameters: {
    "id": string;
    "body": CreateSessionGrantRequest;
  },
  options: CallOptions = {},
): Promise<SessionGrantSummary> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => CreateSessionGrantRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/credential-grants", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => SessionGrantSummarySchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

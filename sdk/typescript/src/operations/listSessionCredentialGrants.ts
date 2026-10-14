// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {SessionGrantListResponse} from "../models/SessionGrantListResponse.js";
import {SessionGrantListResponseSchema} from "../validation/SessionGrantListResponse.js";

export async function listSessionCredentialGrants(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<SessionGrantListResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/credential-grants", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => SessionGrantListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {RevokeSessionGrantResponse} from "../models/RevokeSessionGrantResponse.js";
import {RevokeSessionGrantResponseSchema} from "../validation/RevokeSessionGrantResponse.js";

export async function revokeSessionCredentialGrant(
  transport: Transport,
  parameters: {
    "id": string;
    "grant_id": string;
  },
  options: CallOptions = {},
): Promise<RevokeSessionGrantResponse> {
  const input = z.object({
  "id": z.string(),
  "grant_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.DELETE, "/vms/{id}/credential-grants/{grant_id}", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "grant_id": input["grant_id"]},
  });
  return z.lazy(() => RevokeSessionGrantResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

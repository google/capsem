// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {CredentialInjectRequest} from "../models/CredentialInjectRequest.js";
import type {CredentialInjectResponse} from "../models/CredentialInjectResponse.js";
import {CredentialInjectRequestSchema} from "../validation/CredentialInjectRequest.js";
import {CredentialInjectResponseSchema} from "../validation/CredentialInjectResponse.js";

export async function injectCredential(
  transport: Transport,
  parameters: {
    "body": CredentialInjectRequest;
  },
  options: CallOptions = {},
): Promise<CredentialInjectResponse> {
  const input = z.object({
  "body": z.lazy(() => CredentialInjectRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/credentials/inject", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => CredentialInjectResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

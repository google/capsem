// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {AuthorizeGoogleRequest} from "../models/AuthorizeGoogleRequest.js";
import type {AuthorizeGoogleResponse} from "../models/AuthorizeGoogleResponse.js";
import {AuthorizeGoogleRequestSchema} from "../validation/AuthorizeGoogleRequest.js";
import {AuthorizeGoogleResponseSchema} from "../validation/AuthorizeGoogleResponse.js";

export async function authorizeGoogleConnection(
  transport: Transport,
  parameters: {
    "body": AuthorizeGoogleRequest;
  },
  options: CallOptions = {},
): Promise<AuthorizeGoogleResponse> {
  const input = z.object({
  "body": z.lazy(() => AuthorizeGoogleRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/connections/google/authorize", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => AuthorizeGoogleResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ImagePullRequest} from "../models/ImagePullRequest.js";
import type {ImagePullResponse} from "../models/ImagePullResponse.js";
import {ImagePullRequestSchema} from "../validation/ImagePullRequest.js";
import {ImagePullResponseSchema} from "../validation/ImagePullResponse.js";

export async function pullImage(
  transport: Transport,
  parameters: {
    "body": ImagePullRequest;
  },
  options: CallOptions = {},
): Promise<ImagePullResponse> {
  const input = z.object({
  "body": z.lazy(() => ImagePullRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/images/pull", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ImagePullResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

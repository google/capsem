// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ImageListResponse} from "../models/ImageListResponse.js";
import {ImageListResponseSchema} from "../validation/ImageListResponse.js";

export async function listImages(
  transport: Transport,
  parameters: {
    "refresh"?: boolean;
  } = {},
  options: CallOptions = {},
): Promise<ImageListResponse> {
  const input = z.object({
  "refresh": z.boolean().exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/images", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    query: {"refresh": input["refresh"]},
  });
  return z.lazy(() => ImageListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

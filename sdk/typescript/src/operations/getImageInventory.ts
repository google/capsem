// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ImageInventoryResponse} from "../models/ImageInventoryResponse.js";
import {ImageInventoryResponseSchema} from "../validation/ImageInventoryResponse.js";

export async function getImageInventory(
  transport: Transport,
  parameters: {
    "name": string;
    "image": string;
  },
  options: CallOptions = {},
): Promise<ImageInventoryResponse> {
  const input = z.object({
  "name": z.string(),
  "image": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/images/{name}/inventory", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"name": input["name"]},
    query: {"image": input["image"]},
  });
  return z.lazy(() => ImageInventoryResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}

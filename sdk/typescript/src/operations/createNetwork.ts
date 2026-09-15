// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {CreateNetworkRequest} from "../models/CreateNetworkRequest.js";
import type {NetworkInfo} from "../models/NetworkInfo.js";
import {CreateNetworkRequestSchema} from "../validation/CreateNetworkRequest.js";
import {NetworkInfoSchema} from "../validation/NetworkInfo.js";

export async function createNetwork(
  transport: Transport,
  parameters: {
    "body": CreateNetworkRequest;
  },
  options: CallOptions = {},
): Promise<NetworkInfo> {
  const input = z.object({
  "body": z.lazy(() => CreateNetworkRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/networks", {
    signal: options.signal, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => NetworkInfoSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
